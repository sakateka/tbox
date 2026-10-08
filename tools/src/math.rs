use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use rustpython_parser::{Parse, ast};

// Only evaluate pure numeric expressions here. Everything else stays with Python.
// Bound the work before falling back, including powers and deeply nested inputs.
const MAX_BITS: u64 = 4096;

enum Number {
    Int(BigInt),
    Float(f64),
}

impl Number {
    fn float(&self) -> Option<f64> {
        match self {
            Self::Int(n) => n.to_f64().filter(|n| n.is_finite()),
            Self::Float(n) => Some(*n),
        }
    }

    fn checked(self) -> Option<Self> {
        match &self {
            Self::Int(n) if n.bits() <= MAX_BITS => Some(self),
            Self::Float(n) if n.is_finite() => Some(self),
            _ => None,
        }
    }

    fn render(self) -> String {
        match self {
            Self::Int(n) => n.to_string(),
            Self::Float(n) => {
                // Python uses scientific notation below 1e-4 and from 1e16,
                // with a signed exponent containing at least two digits.
                let scientific = format!("{n:e}");
                let (mantissa, exponent) = scientific.split_once('e').unwrap();
                let exponent: i32 = exponent.parse().unwrap();
                if !(-4..16).contains(&exponent) {
                    format!("{mantissa}e{exponent:+03}")
                } else if n.fract() == 0.0 {
                    format!("{n:.1}")
                } else {
                    n.to_string()
                }
            }
        }
    }
}

fn binary(left: Number, op: ast::Operator, right: Number) -> Option<Number> {
    use ast::Operator as Op;
    if let (Number::Int(a), Number::Int(b)) = (&left, &right) {
        let integer = match op {
            Op::Add => Some(a + b),
            Op::Sub => Some(a - b),
            Op::Mult => Some(a * b),
            Op::FloorDiv | Op::Mod if !b.is_zero() => {
                let mut quotient = a / b;
                let mut remainder = a % b;
                if !remainder.is_zero() && a.is_negative() != b.is_negative() {
                    quotient -= 1;
                    remainder += b;
                }
                Some(if op == Op::FloorDiv {
                    quotient
                } else {
                    remainder
                })
            }
            Op::Pow if !b.is_negative() => {
                let exponent = b.to_u32()?;
                if a.bits().saturating_mul(exponent as u64) > MAX_BITS {
                    return None;
                }
                Some(a.pow(exponent))
            }
            Op::BitAnd => Some(a & b),
            Op::BitOr => Some(a | b),
            Op::BitXor => Some(a ^ b),
            Op::LShift | Op::RShift => {
                let shift = b.to_u32()?;
                if u64::from(shift) > MAX_BITS {
                    return None;
                }
                Some(if op == Op::LShift {
                    a << shift
                } else {
                    a >> shift
                })
            }
            Op::Div if a.bits() > 53 || b.bits() > 53 => return None,
            Op::Pow if a.bits() > 53 || b.bits() > 53 => return None,
            _ => None,
        };
        if let Some(n) = integer {
            return Number::Int(n).checked();
        }
    }
    let (a, b) = (left.float()?, right.float()?);
    let n = match op {
        Op::Add => a + b,
        Op::Sub => a - b,
        Op::Mult => a * b,
        Op::Div if b != 0.0 => a / b,
        Op::Pow if a >= 0.0 || b.fract() == 0.0 => a.powf(b),
        // Python's floating point // and % have additional rounding rules.
        _ => return None,
    };
    Number::Float(n).checked()
}

fn evaluate_node(expr: &ast::Expr, depth: usize) -> Option<Number> {
    if depth > 64 {
        return None;
    }
    let eval = |expr| evaluate_node(expr, depth + 1);
    match expr {
        ast::Expr::Constant(node) => match &node.value {
            ast::Constant::Int(n) => Number::Int(n.clone()).checked(),
            ast::Constant::Float(n) => Number::Float(*n).checked(),
            _ => None,
        },
        ast::Expr::BinOp(node) => binary(eval(&node.left)?, node.op, eval(&node.right)?),
        ast::Expr::UnaryOp(node) => match (node.op, eval(&node.operand)?) {
            (ast::UnaryOp::UAdd, n) => Some(n),
            (ast::UnaryOp::USub, Number::Int(n)) => Some(Number::Int(-n)),
            (ast::UnaryOp::USub, Number::Float(n)) => Some(Number::Float(-n)),
            (ast::UnaryOp::Invert, Number::Int(n)) => Some(Number::Int(!n)),
            _ => None,
        },
        ast::Expr::Name(node) => match node.id.as_str() {
            "pi" => Some(Number::Float(std::f64::consts::PI)),
            "e" => Some(Number::Float(std::f64::consts::E)),
            "tau" => Some(Number::Float(std::f64::consts::TAU)),
            _ => None,
        },
        ast::Expr::Call(node) if node.keywords.is_empty() => {
            let ast::Expr::Name(function) = node.func.as_ref() else {
                return None;
            };
            let [argument] = node.args.as_slice() else {
                return None;
            };
            let argument = eval(argument)?;
            if function.id.as_str() == "abs" {
                return match argument {
                    Number::Int(n) => Some(Number::Int(n.abs())),
                    Number::Float(n) => Some(Number::Float(n.abs())),
                };
            }
            let n = argument.float()?;
            let n = match function.id.as_str() {
                "sqrt" => n.sqrt(),
                "sin" => n.sin(),
                "cos" => n.cos(),
                "tan" => n.tan(),
                "asin" => n.asin(),
                "acos" => n.acos(),
                "atan" => n.atan(),
                "exp" => n.exp(),
                "expm1" => n.exp_m1(),
                "log" => n.ln(),
                "log2" => n.log2(),
                "log10" => n.log10(),
                "log1p" => n.ln_1p(),
                "sinh" => n.sinh(),
                "cosh" => n.cosh(),
                "tanh" => n.tanh(),
                _ => return None,
            };
            Number::Float(n).checked()
        }
        _ => None,
    }
}

pub fn evaluate(expression: &str) -> Option<String> {
    // The Python fallback wraps the input in print(...), so comments can change
    // how its closing parenthesis is parsed. Leave those inputs to Python too.
    if expression.len() > 4096 || expression.contains('#') {
        return None;
    }
    let mut nesting = 0_usize;
    for c in expression.chars() {
        match c {
            '(' | '[' | '{' => nesting += 1,
            ')' | ']' | '}' => nesting = nesting.saturating_sub(1),
            _ => {}
        }
        if nesting > 64 {
            return None;
        }
    }
    let expr = ast::Expr::parse(expression.trim(), "<expression>").ok()?;
    Some(evaluate_node(&expr, 0)?.render())
}

#[cfg(test)]
mod tests {
    use super::evaluate;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    #[test]
    fn numeric_results_match_python() {
        let mut expressions: Vec<String> = [
            "1+2",
            "1/2",
            "4/2",
            "2**8",
            "2**256",
            "2**-3",
            "-2**2",
            "2**3**2",
            "-7//3",
            "7//-3",
            "-7%3",
            "7%-3",
            "abs(-123)",
            "sqrt(9)",
            "sin(pi/2)",
            "cos(pi)",
            "log(10)",
            "log10(100)",
            "log2(8)",
            "exp(1)",
            "tau",
            "-0.0",
            "sin(-0.0)",
            "1e-5",
            "1e16",
            "1e15",
            "1e-4",
            "1.23456789e-7",
            "1e300",
            "1e-300",
            "1.0+2.0",
            "1_000+0xff",
            "0b100 << 20",
            "~0",
            "(-5)^3",
            "9007199254740993+1",
            "100000000000000000000-1",
            "(-2)**-3",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        for a in [-21, -3, -1, 1, 3, 7, 11, 100] {
            for b in [-7, -2, 1, 3, 7] {
                for op in ["+", "-", "*", "/", "//", "%", "**"] {
                    expressions.push(format!("({a}){op}({b})"));
                }
                for op in ["+", "-", "*", "/"] {
                    expressions.push(format!("({a}.25){op}({b}.75)"));
                }
            }
        }
        for n in [0.0, 0.00001, 0.01, 0.3, 1.0, 1.5, 10.0, 100.0] {
            for function in [
                "sqrt", "sin", "cos", "tan", "atan", "sinh", "cosh", "tanh", "expm1", "log1p",
            ] {
                expressions.push(format!("{function}({n:?})"));
            }
        }
        let mut python = Command::new("python3")
            .args([
                "-c",
                "from math import *\nimport sys\nfor s in sys.stdin:\n print(eval(s))",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        python
            .stdin
            .take()
            .unwrap()
            .write_all(expressions.join("\n").as_bytes())
            .unwrap();
        let output = python.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = String::from_utf8(output.stdout).unwrap();
        assert_eq!(expected.lines().count(), expressions.len());
        for (expression, expected) in expressions.iter().zip(expected.lines()) {
            assert_eq!(
                evaluate(expression).as_deref(),
                Some(expected),
                "{expression}"
            );
        }
    }

    #[test]
    fn unsupported_expressions_and_errors_fall_back_without_executing() {
        for expression in [
            "sum(range(10))",
            "[x*x for x in range(10)]",
            "__import__('sys').exit(1)",
            "factorial(100)",
            "sqrt(-1)",
            "1/0",
            "0**-1",
            "1.0//0.1",
            "1.0%0.1",
            "(-1)**-9007199254740993",
            "2**1000000000",
            "2**100/3",
            "1+",
            "nan",
            "1 # comment",
        ] {
            assert!(evaluate(expression).is_none(), "{expression}");
        }
        assert!(evaluate(&format!("{}1{}", "(".repeat(2000), ")".repeat(2000))).is_none());
    }
}
