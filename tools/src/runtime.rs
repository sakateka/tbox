use crate::{config::Config, host};
use anyhow::{Context, Result};
use starlark::{
    ErrorKind,
    environment::{GlobalsBuilder, Module},
    eval::Evaluator,
    syntax::{AstModule, Dialect},
};

fn script_error(error: starlark::Error, filename: &str) -> anyhow::Error {
    let message = match error.kind() {
        ErrorKind::Fail(cause) | ErrorKind::Native(cause) | ErrorKind::Other(cause) => {
            format!("{cause:#}").trim_start().to_owned()
        }
        _ => error.without_diagnostic().to_string(),
    };
    let location = match error.span() {
        Some(span) => {
            let pos = span.resolve_span().begin;
            format!("{}:{}:{}", span.filename(), pos.line + 1, pos.column + 1)
        }
        None => filename.to_owned(),
    };
    anyhow::anyhow!("{message}\n  at {location}")
}

pub fn run(config: &Config, name: &str, args: &[String]) -> Result<()> {
    let applet = config.applets.get(name).context("Unknown applet")?;
    let filename = format!("{}[applets.{name}.script]", config.source.display());
    let ast = AstModule::parse(&filename, applet.script.clone(), &Dialect::Standard)
        .map_err(|e| script_error(e, &filename))?;
    let globals = GlobalsBuilder::standard().with(host::functions).build();
    Module::with_temp_heap(|module| {
        let mut evaluator = Evaluator::new(&module);
        evaluator.extra = Some(config);
        evaluator
            .eval_module(ast, &globals)
            .map_err(|e| script_error(e, &filename))?;
        let main = module
            .get("main")
            .with_context(|| format!("Script must define main(args)\n  at {filename}"))?;
        evaluator
            .eval_function(main, &[module.heap().alloc(args.to_vec())], &[])
            .map_err(|e| script_error(e, &filename))?;
        Ok::<(), anyhow::Error>(())
    })
}
