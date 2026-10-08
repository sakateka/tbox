mod config;
mod host;
mod http;
mod runtime;

use anyhow::{Result, bail};
use config::Config;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn print_applets(cfg: &Config) {
    println!(
        "Usage: tbox [--config PATH] <applet> [args...]\n       <alias> [args...]\n\nAvailable applets:"
    );
    let mut commands = BTreeMap::new();
    for (alias, target) in &cfg.aliases {
        if let Some(applet) = cfg.applets.get(target) {
            commands.insert(alias, applet);
        }
    }
    for (name, applet) in &cfg.applets {
        if !cfg.aliases.values().any(|target| target == name) {
            commands.entry(name).or_insert(applet);
        }
    }
    for (name, applet) in commands {
        println!(
            "  {:<24} {}",
            name,
            applet.description.lines().next().unwrap_or_default()
        );
    }
    if cfg.applets.is_empty() {
        println!(
            "  No applets configured. Define [applets.NAME] with description, usage and script in {}.",
            cfg.source.display()
        );
        println!("  Select another config with --config PATH / TBOX_CONFIG.");
    }
    println!("\nRun 'tbox <applet> --help' for applet help.");
}

fn run() -> Result<()> {
    let mut argv = std::env::args_os()
        .map(|arg| {
            arg.into_string()
                .map_err(|_| anyhow::anyhow!("Command-line arguments must be valid UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter();
    let invocation = argv.next().unwrap_or_else(|| "tbox".into());
    let executable = Path::new(&invocation)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&invocation);
    let mut args: Vec<String> = argv.collect();
    let mut config_path = None;
    if executable == "tbox" {
        if args.first().is_some_and(|arg| arg == "--config") {
            if args.len() < 2 {
                bail!("--config requires a path");
            }
            config_path = Some(PathBuf::from(args.remove(1)));
            args.remove(0);
        } else if args.first().is_some_and(|arg| arg.starts_with("--config=")) {
            config_path = Some(PathBuf::from(
                args.remove(0).trim_start_matches("--config="),
            ));
        }
    }
    let cfg = Config::load(config_path)?;
    let name = if executable == "tbox" {
        if args.is_empty() || matches!(args[0].as_str(), "--list" | "--help" | "-h") {
            print_applets(&cfg);
            return Ok(());
        }
        args.remove(0)
    } else {
        executable.to_owned()
    };
    let target = cfg
        .aliases
        .get(&name)
        .or_else(|| cfg.aliases.get(&invocation))
        .unwrap_or(&name);
    let meta = cfg.applets.get(target).ok_or_else(|| {
        anyhow::anyhow!("Unknown applet '{target}'. Run 'tbox --list' to see available applets.")
    })?;
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        println!("{}\n\nUsage: tbox {}", meta.description, meta.usage);
        return Ok(());
    }
    runtime::run(&cfg, target, &args)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("tbox: {error:#}");
        std::process::exit(1);
    }
}
