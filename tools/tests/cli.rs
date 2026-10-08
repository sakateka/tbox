use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Output},
    thread,
    time::Duration,
};
use tempfile::TempDir;

struct Fixture {
    temp: TempDir,
    config: PathBuf,
}
impl Fixture {
    fn new(extra: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        fs::write(&config, extra).unwrap();
        Self { temp, config }
    }
    fn script(&self, name: &str, source: &str) {
        let mut cfg: toml::Table =
            toml::from_str(&fs::read_to_string(&self.config).unwrap()).unwrap();
        let applets = cfg
            .entry("applets")
            .or_insert_with(|| toml::Table::new().into());
        applets.as_table_mut().unwrap().insert(
            name.into(),
            toml::Table::from_iter([
                (
                    "description".into(),
                    format!("Synthetic {name} applet.").into(),
                ),
                ("usage".into(), format!("{name} <value>").into()),
                ("script".into(), source.into()),
            ])
            .into(),
        );
        fs::write(&self.config, toml::to_string(&cfg).unwrap()).unwrap();
    }
    fn configure(&self, extra: &str) {
        let old: toml::Table = toml::from_str(&fs::read_to_string(&self.config).unwrap()).unwrap();
        let mut cfg: toml::Table = toml::from_str(extra).unwrap();
        if let Some(applets) = old.get("applets") {
            cfg.insert("applets".into(), applets.clone());
        }
        fs::write(&self.config, toml::to_string(&cfg).unwrap()).unwrap();
    }
    fn write(&self, path: &str, text: &str) {
        fs::write(self.temp.path().join(path), text).unwrap();
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_tbox"));
        cmd.arg("--config").arg(&self.config);
        cmd.env_remove("TBOX_CONFIG");
        cmd.env("NO_PROXY", "localhost,127.0.0.1");
        cmd
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
}
fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn failure(output: Output, expected: &str) -> String {
    assert!(!output.status.success(), "unexpected success: {:?}", output);
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains(expected), "{error}");
    assert!(!error.contains("panicked"), "{error}");
    error
}

#[test]
fn inline_toml_example_runs_without_other_files() {
    let f = Fixture::new(include_str!("../../tbox.example.toml"));
    assert_eq!(success(f.run(&["greet", "World"])), "Hello, World!\n");
    assert!(success(f.run(&["--list"])).contains("greet"));
    assert!(success(f.run(&["lookup", "--help"])).contains("lookup <value>"));
    let entries: Vec<_> = fs::read_dir(f.temp.path()).unwrap().collect();
    assert_eq!(entries.len(), 1, "Only the TOML config is needed");
}

#[test]
fn math_avoids_python_for_arithmetic_and_preserves_python_fallback() {
    let f = Fixture::new(include_str!("../../tbox.example.toml"));
    f.configure("[commands]\npython = ['/missing/python-must-not-be-started']\n");
    for (expression, expected) in [
        ("1+2", "3\n"),
        ("1/2", "0.5\n"),
        ("2**8", "256\n"),
        ("sqrt(9)", "3.0\n"),
        ("sin(pi/2)", "1.0\n"),
    ] {
        assert_eq!(success(f.run(&["calc", expression])), expected);
    }
    failure(
        f.run(&["calc", "sum(range(10))"]),
        "Cannot execute command binding 'python'",
    );
    f.configure("[commands]\npython = ['python3']\n");
    for (expression, expected) in [
        ("sum(range(10))", "45\n"),
        ("[x*x for x in range(3)]", "[0, 1, 4]\n"),
        ("__import__('decimal').Decimal('0.1')*3", "0.3\n"),
    ] {
        assert_eq!(success(f.run(&["calc", expression])), expected);
    }
    assert_eq!(success(f.run(&["calc", "1", "/", "2"])), "0.5\n");
    let error = failure(f.run(&["calc"]), "Expected an expression");
    assert!(error.contains("Usage: tbox calc <expression...>"));
}

#[cfg(unix)]
#[test]
fn regression_executable_alias_forwards_config_arguments() {
    let f = Fixture::new("[aliases]\nshort = 'echoer'\n");
    f.script("echoer", "def main(args):\n    emit('|'.join(args))\n");
    let alias = f.temp.path().join("short");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tbox"), &alias).unwrap();
    for args in [&["--config", "literal"][..], &["--config=literal"]] {
        let direct = success(f.command().arg("echoer").args(args).output().unwrap());
        let aliased = success(
            Command::new(&alias)
                .env("TBOX_CONFIG", &f.config)
                .args(args)
                .output()
                .unwrap(),
        );
        assert_eq!(direct, args.join("|"));
        assert_eq!(aliased, direct);
    }
    fs::create_dir_all(f.temp.path().join(".config/tbox")).unwrap();
    fs::copy(&f.config, f.temp.path().join(".config/tbox/config.toml")).unwrap();
    assert_eq!(
        success(
            Command::new(alias)
                .env_remove("TBOX_CONFIG")
                .env("HOME", f.temp.path())
                .args(["--config", "literal"])
                .output()
                .unwrap()
        ),
        "--config|literal"
    );
}

#[cfg(unix)]
#[test]
fn regression_invalid_utf8_arguments_return_a_normal_error() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt, process::CommandExt},
    };
    let f = Fixture::new("");
    f.script("echoer", "def main(args):\n    emit('|'.join(args))\n");
    let output = f
        .command()
        .arg("echoer")
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    failure(output, "valid UTF-8");
    let output = f
        .command()
        .arg0(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    failure(output, "valid UTF-8");
}

#[test]
fn regression_url_dot_only_path_substitutions_are_rejected() {
    let f = Fixture::new(
        "[links.item]\nurl = 'https://example.com/items/{value}/detail'\ncommand = 'unused'\n[links.tree]\nurl = 'https://example.com/items/{value:path}/detail'\ncommand = 'unused'\n[links.dots]\nurl = 'https://example.com/items/detail?q={value}#{value}'\ncommand = 'unused'\n",
    );
    f.script(
        "render",
        "def main(args):\n    emit(render_url(args[0], {'value': args[1]}))\n",
    );
    for profile in ["item", "tree"] {
        for value in [".", ".."] {
            failure(f.run(&["render", profile, value]), "dot-only path segments");
        }
    }
    for value in ["./child", "parent/../child", "parent/.", "parent/.."] {
        failure(f.run(&["render", "tree", value]), "dot-only path segments");
    }
    assert_eq!(
        success(f.run(&["render", "item", "a..b"])),
        "https://example.com/items/a..b/detail"
    );
    assert_eq!(
        success(f.run(&["render", "tree", "a..b/file.txt"])),
        "https://example.com/items/a..b/file.txt/detail"
    );
    for value in [".", ".."] {
        assert_eq!(
            success(f.run(&["render", "dots", value])),
            format!("https://example.com/items/detail?q={value}#{value}")
        );
    }
}

#[test]
fn discovery_help_aliases_and_changes_need_no_rebuild() {
    let f = Fixture::new("[aliases]\nshort = 'echoer'\nalternate = 'echoer'\n");
    f.script("zebra", "fail('must never evaluate for help')\n");
    f.script("echoer", "def main(args):\n    emit('|'.join(args))\n");
    for args in [&[][..], &["--list"], &["--help"], &["-h"]] {
        let listed = success(f.run(args));
        let names: Vec<_> = listed
            .lines()
            .filter(|line| line.starts_with("  ") && line.contains("Synthetic"))
            .map(|line| line.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(names, ["short", "zebra"]);
        assert_eq!(listed.matches("Synthetic echoer applet.").count(), 1);
        assert!(!listed.contains("alternate"));
    }
    for args in [["zebra", "--help"], ["short", "-h"]] {
        let text = success(f.run(&args));
        assert!(text.contains("Synthetic"));
        assert!(text.contains("Usage: tbox"));
    }
    assert_eq!(success(f.run(&["echoer", "a b", "c/d"])), "a b|c/d");
    assert_eq!(success(f.run(&["short", "a b", "c/d"])), "a b|c/d");
    #[cfg(unix)]
    {
        let alias = f.temp.path().join("short");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tbox"), &alias).unwrap();
        assert_eq!(
            success(
                Command::new(alias)
                    .env("TBOX_CONFIG", &f.config)
                    .args(["a b", "c/d"])
                    .output()
                    .unwrap()
            ),
            "a b|c/d"
        );
    }
    f.script("echoer", "def main(args):\n    emit('updated')\n");
    assert_eq!(success(f.run(&["echoer"])), "updated");
    f.script("newone", "def main(args):\n    emit('new')\n");
    assert_eq!(success(f.run(&["newone"])), "new");
}

#[test]
fn listing_prefers_an_alias_even_when_the_applet_name_is_shorter() {
    let f = Fixture::new("[aliases]\nalternate = 'x'\npreferred = 'x'\n");
    f.script("x", "def main(args):\n    emit('ok')\n");
    let listed = success(f.run(&["--list"]));
    let names: Vec<_> = listed
        .lines()
        .filter(|line| line.starts_with("  ") && line.contains("Synthetic"))
        .map(|line| line.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(names, ["alternate"]);
    assert!(!listed.contains("preferred"));
    for name in ["x", "alternate", "preferred"] {
        assert_eq!(success(f.run(&[name])), "ok");
    }
}

#[test]
fn config_selection_empty_installation_and_invalid_inputs() {
    let f = Fixture::new("");
    let mut default = Command::new(env!("CARGO_BIN_EXE_tbox"));
    default.env("HOME", f.temp.path()).env_remove("TBOX_CONFIG");
    assert!(success(default.output().unwrap()).contains("No applets configured"));
    f.script("sample", "def main(args):\n    emit('configured')\n");
    assert_eq!(
        success(
            f.command()
                .env("TBOX_CONFIG", "/missing/ignored-config")
                .arg("sample")
                .output()
                .unwrap()
        ),
        "configured"
    );
    assert_eq!(
        success(
            Command::new(env!("CARGO_BIN_EXE_tbox"))
                .arg(format!("--config={}", f.config.display()))
                .arg("sample")
                .output()
                .unwrap()
        ),
        "configured"
    );
    assert_eq!(
        success(
            Command::new(env!("CARGO_BIN_EXE_tbox"))
                .env("TBOX_CONFIG", &f.config)
                .arg("sample")
                .output()
                .unwrap()
        ),
        "configured"
    );
    failure(f.run(&["unknown"]), "Unknown applet");
    failure(
        Command::new(env!("CARGO_BIN_EXE_tbox"))
            .arg("--config")
            .output()
            .unwrap(),
        "requires a path",
    );
    failure(
        Command::new(env!("CARGO_BIN_EXE_tbox"))
            .args(["--config", "/nonexistent/synthetic.toml"])
            .output()
            .unwrap(),
        "Cannot read config",
    );
    fs::write(
        &f.config,
        "[applets.sample]\nusage = 'sample'\nscript = 'def main(args): pass'\n",
    )
    .unwrap();
    failure(f.run(&["--list"]), "description");
    fs::write(&f.config, "[applets.sample]\ndescription = 'one'\nusage = 'sample'\nscript = 'pass'\n[applets.sample]\ndescription = 'duplicate'\n").unwrap();
    failure(f.run(&["--list"]), "duplicate");
    for (name, field) in [
        ("invalid name", "description"),
        ("sample", "description"),
        ("sample", "usage"),
        ("sample", "script"),
    ] {
        let mut applet = toml::Table::from_iter([
            ("description".into(), "Example".into()),
            ("usage".into(), "sample".into()),
            ("script".into(), "def main(args): pass".into()),
        ]);
        if name == "sample" {
            applet.insert(field.into(), "  ".into());
        }
        let cfg = toml::Table::from_iter([(
            "applets".into(),
            toml::Table::from_iter([(name.into(), applet.into())]).into(),
        )]);
        fs::write(&f.config, toml::to_string(&cfg).unwrap()).unwrap();
        failure(f.run(&["--list"]), "Invalid applet");
    }
}

#[test]
fn script_errors_include_locations_and_help_never_parses_scripts() {
    let f = Fixture::new("");
    f.script("broken", "def main(args):\n    invalid syntax !\n");
    success(f.run(&["broken", "--help"]));
    let error = failure(f.run(&["broken"]), "config.toml[applets.broken.script]");
    assert!(error.contains(":2"));
    assert!(error.contains("Synthetic broken applet.\n\nUsage: tbox broken <value>"));
    assert!(!error.contains("Traceback"), "{error}");
    f.script("broken", "def main(args):\n    fail('synthetic failure')\n");
    let error = failure(f.run(&["broken"]), "synthetic failure");
    assert!(
        error.contains("config.toml[applets.broken.script]:2"),
        "{error}"
    );
    assert!(!error.contains("Traceback"), "{error}");
    assert!(!error.contains("fail:"), "{error}");
    f.script("broken", "def main(args):\n    emit(1 // 0)\n");
    let error = failure(f.run(&["broken"]), "config.toml[applets.broken.script]:2");
    assert!(error.contains("Usage: tbox broken <value>"), "{error}");
    assert!(!error.contains("Traceback"), "{error}");
    f.script("broken", "def other(args):\n    pass\n");
    let error = failure(f.run(&["broken"]), "main(args)");
    assert!(error.contains("config.toml[applets.broken.script]"));
    assert!(error.contains("Usage: tbox broken <value>"));
    f.script("broken", "fail('module failure')\n");
    let error = failure(f.run(&["broken"]), "module failure");
    assert!(error.contains("config.toml[applets.broken.script]:1:1"));
    assert!(error.contains("Usage: tbox broken <value>"));
    assert!(!error.contains("Traceback"), "{error}");
}

#[test]
fn invalid_arguments_show_reason_location_description_and_invoked_usage() {
    let f = Fixture::new("[aliases]\nshort = 'sample'\n");
    f.script(
        "sample",
        "def validate(args):\n    if len(args) != 1:\n        fail('Expected exactly one argument; got ' + str(len(args)))\ndef main(args):\n    validate(args)\n    emit(args[0])\n",
    );
    for name in ["sample", "short"] {
        for args in [vec![name], vec![name, "one", "two"]] {
            let output = f.run(&args);
            assert_eq!(output.status.code(), Some(1));
            assert!(output.stdout.is_empty());
            let error = String::from_utf8(output.stderr).unwrap();
            assert_eq!(
                error,
                format!(
                    "tbox: {name}: Expected exactly one argument; got {}\n  at {}[applets.sample.script]:3:9\n\nSynthetic sample applet.\n\nUsage: tbox {name} <value>\n",
                    args.len() - 1,
                    f.config.display()
                )
            );
        }
        assert_eq!(success(f.run(&[name, "value"])), "value");
        assert_eq!(
            success(f.run(&[name, "--help"])),
            format!("Synthetic sample applet.\n\nUsage: tbox {name} <value>\n")
        );
    }

    #[cfg(unix)]
    {
        let alias = f.temp.path().join("short");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_tbox"), &alias).unwrap();
        let run = |args: &[&str]| {
            Command::new(&alias)
                .env("TBOX_CONFIG", &f.config)
                .args(args)
                .output()
                .unwrap()
        };
        let error = failure(
            run(&[]),
            "tbox: short: Expected exactly one argument; got 0",
        );
        assert!(error.ends_with("Usage: short <value>\n"), "{error}");
        assert!(!error.contains("Traceback"), "{error}");
        assert_eq!(
            success(run(&["--help"])),
            "Synthetic sample applet.\n\nUsage: short <value>\n"
        );
    }
}

fn read_request(stream: &mut impl Read) -> String {
    let mut buf = Vec::new();
    let mut byte = [0];
    loop {
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            break;
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let head = String::from_utf8(buf.clone()).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|n| n.trim().parse().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).unwrap();
    buf.extend(body);
    String::from_utf8(buf).unwrap()
}
fn response(stream: &mut impl Write, status: &str, text: &str) {
    let message = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.write_all(message.as_bytes());
}
fn http_server(
    status: &'static str,
    body: &'static str,
    delay: Duration,
) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let req = read_request(&mut socket);
        thread::sleep(delay);
        response(&mut socket, status, body);
        req
    });
    (url, handle)
}

#[test]
fn http_exact_body_encoding_oauth_and_configured_method_body_headers() {
    let (url, server) = http_server("200 OK", "synthetic body\n\n", Duration::ZERO);
    let f = Fixture::new(&format!(
        "[http.query]\nurl = '{url}/api/{{value}}?q={{value}}'\nmethod = 'POST'\nbody = 'request body'\ntoken_env = 'TBOX_SYNTHETIC_TOKEN'\n[http.query.headers]\nX-Example = 'present'\n"
    ));
    f.script(
        "fetch",
        "def main(args):\n    emit(http_request('query', {'value': args[0]}))\n",
    );
    assert_eq!(
        success(
            f.command()
                .env("TBOX_SYNTHETIC_TOKEN", "synthetic-secret")
                .args(["fetch", "a /%?#"])
                .output()
                .unwrap()
        ),
        "synthetic body\n\n"
    );
    let request = server.join().unwrap();
    assert!(
        request.starts_with("POST /api/a%20%2F%25%3F%23?q=a%20%2F%25%3F%23 HTTP/1.1\r\n"),
        "{request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: oauth synthetic-secret\r\n")
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-example: present\r\n")
    );
    assert!(request.ends_with("\r\n\r\nrequest body"));
}

#[test]
fn http_token_errors_status_and_timeout_are_redacted() {
    let f = Fixture::new(
        "[http.query]\nurl = 'http://127.0.0.1:9/{value}'\ntoken_env = 'TBOX_SYNTHETIC_TOKEN'\n",
    );
    f.script(
        "fetch",
        "def main(args):\n    emit(http_request('query', {'value': 'hidden-url-value'}))\n",
    );
    failure(
        f.command()
            .env_remove("TBOX_SYNTHETIC_TOKEN")
            .arg("fetch")
            .output()
            .unwrap(),
        "token environment variable",
    );
    let error = failure(
        f.command()
            .env("TBOX_SYNTHETIC_TOKEN", "secret\ninvalid")
            .arg("fetch")
            .output()
            .unwrap(),
        "token is invalid",
    );
    assert!(!error.contains("secret"));
    f.configure("[http.query]\nurl = 'http://127.0.0.1:9/{value}'\ntimeout_secs = 0\n");
    failure(f.run(&["fetch"]), "timeout_secs must be between");
    let (url, server) = http_server("401 Unauthorized", "rejected", Duration::ZERO);
    f.configure(&format!(
        "[http.query]\nurl = '{url}/{{value}}'\ntoken_file = 'token'\n"
    ));
    f.write("token", "wrong-secret\n");
    let error = failure(f.run(&["fetch"]), "status 401");
    assert!(!error.contains("wrong-secret"));
    assert!(server.join().unwrap().contains("OAuth wrong-secret"));
    let (url, server) = http_server("200 OK", "too late", Duration::from_secs(2));
    f.configure(&format!(
        "[http.query]\nurl = '{url}/{{value}}'\ntimeout_secs = 1\n"
    ));
    failure(f.run(&["fetch"]), "timed out");
    server.join().unwrap();
}

fn tls_material() -> (
    openssl::x509::X509,
    openssl::pkey::PKey<openssl::pkey::Private>,
    openssl::x509::X509,
) {
    use openssl::{
        asn1::Asn1Time,
        hash::MessageDigest,
        pkey::PKey,
        rsa::Rsa,
        x509::{
            X509, X509NameBuilder,
            extension::{BasicConstraints, KeyUsage, SubjectAlternativeName},
        },
    };
    let ca_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "Synthetic test CA")
        .unwrap();
    let name = name.build();
    let mut ca = X509::builder().unwrap();
    ca.set_version(2).unwrap();
    ca.set_subject_name(&name).unwrap();
    ca.set_issuer_name(&name).unwrap();
    ca.set_pubkey(&ca_key).unwrap();
    ca.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    ca.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    ca.append_extension(BasicConstraints::new().critical().ca().build().unwrap())
        .unwrap();
    ca.append_extension(KeyUsage::new().key_cert_sign().crl_sign().build().unwrap())
        .unwrap();
    ca.sign(&ca_key, MessageDigest::sha256()).unwrap();
    let ca = ca.build();
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut leaf_name = X509NameBuilder::new().unwrap();
    leaf_name.append_entry_by_text("CN", "localhost").unwrap();
    let leaf_name = leaf_name.build();
    let mut leaf = X509::builder().unwrap();
    leaf.set_version(2).unwrap();
    leaf.set_subject_name(&leaf_name).unwrap();
    leaf.set_issuer_name(ca.subject_name()).unwrap();
    leaf.set_pubkey(&key).unwrap();
    leaf.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    leaf.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    let san = SubjectAlternativeName::new()
        .dns("localhost")
        .build(&leaf.x509v3_context(Some(&ca), None))
        .unwrap();
    leaf.append_extension(san).unwrap();
    leaf.sign(&ca_key, MessageDigest::sha256()).unwrap();
    (ca, key, leaf.build())
}

#[test]
fn https_custom_pem_bundle_keeps_certificate_and_hostname_validation() {
    use openssl::ssl::{SslAcceptor, SslMethod};
    let (ca, key, leaf) = tls_material();
    let f = Fixture::new("");
    fs::write(
        f.temp.path().join("roots.pem"),
        [ca.to_pem().unwrap(), ca.to_pem().unwrap()].concat(),
    )
    .unwrap();
    f.script(
        "fetch",
        "def main(args):\n    emit(http_request('query', {}))\n",
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
    acceptor.set_private_key(&key).unwrap();
    acceptor.set_certificate(&leaf).unwrap();
    let acceptor = acceptor.build();
    let server = thread::spawn(move || {
        for _ in 0..3 {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            if let Ok(mut tls) = acceptor.accept(socket) {
                read_request(&mut tls);
                response(&mut tls, "200 OK", "verified TLS\n");
            }
        }
    });
    let configure = |host: &str, trust: bool| {
        f.configure(&format!(
            "[http.query]\nurl = 'https://{host}:{port}/fixture'\n{}",
            if trust { "ca_file = 'roots.pem'\n" } else { "" }
        ));
    };
    configure("localhost", true);
    assert_eq!(success(f.run(&["fetch"])), "verified TLS\n");
    configure("localhost", false);
    failure(f.run(&["fetch"]), "TLS verification");
    configure("127.0.0.1", true);
    failure(f.run(&["fetch"]), "TLS verification");
    server.join().unwrap();
    configure("localhost", true);
    f.write("roots.pem", "invalid certificate");
    failure(f.run(&["fetch"]), "CA bundle");
}

#[test]
fn links_record_one_encoded_argument_and_relative_paths() {
    let f = Fixture::new(
        "[commands]\nrecord = ['python3', '-c', 'import sys; print(repr(sys.argv[1:]))']\n[links.item]\nurl = 'https://example.com/items/{value}?q={value}#part/{value}'\ncommand = 'record'\n[links.tree]\nurl = 'https://example.com/tree/{path:path}'\ncommand = 'record'\n[roots]\nworkspace = 'workspace'\n",
    );
    f.script(
        "browse",
        "def main(args):\n    open_link('item', {'value': args[0]})\n",
    );
    assert_eq!(
        success(f.run(&["browse", "a /%?#"])),
        "['https://example.com/items/a%20%2F%25%3F%23?q=a%20%2F%25%3F%23#part/a%20%2F%25%3F%23']\n"
    );
    failure(f.run(&["browse"]), "Index");
    f.script("browse", "def main(args):\n    open_link('missing', {})\n");
    assert!(
        failure(f.run(&["browse"]), "Missing link profile")
            .contains("config.toml[applets.browse.script]:2")
    );
    f.script("browse", "def main(args):\n    open_link('item', {})\n");
    let out = f.run(&["browse"]);
    assert!(out.stdout.is_empty());
    failure(out, "Missing URL variable");
    fs::create_dir_all(f.temp.path().join("workspace/nested dir/deep")).unwrap();
    f.script("browse", "def main(args):\n    path = relative_path('workspace')\n    if args:\n        path = path + '/' + args[0] if path else args[0]\n    open_link('tree', {'path': path})\n");
    assert_eq!(
        success(
            f.command()
                .current_dir(f.temp.path().join("workspace"))
                .arg("browse")
                .output()
                .unwrap()
        ),
        "['https://example.com/tree/']\n"
    );
    assert_eq!(
        success(
            f.command()
                .current_dir(f.temp.path().join("workspace/nested dir/deep"))
                .args(["browse", "child ?#%/file"])
                .output()
                .unwrap()
        ),
        "['https://example.com/tree/nested%20dir/deep/child%20%3F%23%25/file']\n"
    );
    failure(f.run(&["browse"]), "outside the configured workspace root");
}

#[test]
fn cryptographic_random_strings_and_python_process_semantics() {
    let f = Fixture::new("[commands]\npython = ['python3']\n");
    f.script("random", "def main(args):\n    size = int(args[0]) if args else 16\n    emit(random_string(size, 'aZ0_!') + '\\n')\n");
    let first = success(f.run(&["random"]));
    let second = success(f.run(&["random"]));
    assert_eq!(first.len(), 17);
    assert!(first.ends_with('\n'));
    assert_ne!(first, second);
    assert!(first.trim_end().chars().all(|c| "aZ0_!".contains(c)));
    assert_eq!(success(f.run(&["random", "0"])), "\n");
    assert_eq!(success(f.run(&["random", "7"])).len(), 8);
    for invalid in ["-1", "bad", "1000001"] {
        failure(
            f.run(&["random", invalid]),
            "config.toml[applets.random.script]",
        );
    }
    f.script("calc", "def main(args):\n    run_command('python', ['-c', 'from math import *; print(' + ' '.join(args) + ')'])\n");
    for (args, expected) in [
        (&["calc", "1", "/", "2"][..], "0.5\n"),
        (&["calc", "2 ** 8"], "256\n"),
        (&["calc", "sqrt(9)"], "3.0\n"),
        (&["calc", "sin(pi/2)"], "1.0\n"),
    ] {
        assert_eq!(success(f.run(args)), expected);
    }
    failure(f.run(&["calc", "1 / 0"]), "exited unsuccessfully");
}
