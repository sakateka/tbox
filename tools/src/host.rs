use crate::{config::Config, http, math};
use anyhow::{Context, Result, bail};
use rand::{Rng, rngs::OsRng};
use starlark::{
    environment::GlobalsBuilder,
    eval::Evaluator,
    starlark_module,
    values::{
        dict::UnpackDictEntries,
        list::UnpackList,
        none::{NoneOr, NoneType},
    },
};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    process::Command,
};

fn config<'a>(eval: &'a Evaluator) -> Result<&'a Config> {
    eval.extra
        .and_then(|extra| extra.downcast_ref::<Config>())
        .context("Missing runtime configuration")
}

fn command(config: &Config, name: &str, args: &[String]) -> Result<()> {
    let binding = config
        .commands
        .get(name)
        .with_context(|| format!("Missing command binding '{name}'"))?;
    let (executable, prefix) = binding.split_first().context("Command binding is empty")?;
    let status = Command::new(executable)
        .args(prefix)
        .args(args)
        .status()
        .with_context(|| format!("Cannot execute command binding '{name}'"))?;
    if !status.success() {
        bail!("Command binding '{name}' exited unsuccessfully ({status})");
    }
    Ok(())
}

#[starlark_module]
pub fn functions(builder: &mut GlobalsBuilder) {
    fn fast_math(expression: &str) -> anyhow::Result<NoneOr<String>> {
        Ok(NoneOr::from_option(math::evaluate(expression)))
    }

    fn emit(text: &str) -> anyhow::Result<NoneType> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(text.as_bytes())?;
        stdout.flush()?;
        Ok(NoneType)
    }

    fn http_request(
        profile: &str,
        variables: UnpackDictEntries<String, String>,
        eval: &mut Evaluator,
    ) -> anyhow::Result<String> {
        http::request(
            config(eval)?,
            profile,
            &variables.entries.into_iter().collect(),
        )
    }

    fn render_url(
        profile: &str,
        variables: UnpackDictEntries<String, String>,
        eval: &mut Evaluator,
    ) -> anyhow::Result<String> {
        let profile = config(eval)?
            .links
            .get(profile)
            .with_context(|| format!("Missing link profile '{profile}'"))?;
        http::render(&profile.url, &variables.entries.into_iter().collect())
    }

    fn open_link(
        profile: &str,
        variables: UnpackDictEntries<String, String>,
        eval: &mut Evaluator,
    ) -> anyhow::Result<NoneType> {
        let cfg = config(eval)?;
        let link = cfg
            .links
            .get(profile)
            .with_context(|| format!("Missing link profile '{profile}'"))?;
        let vars: BTreeMap<_, _> = variables.entries.into_iter().collect();
        let url = http::render(&link.url, &vars)?;
        command(cfg, &link.command, &[url])?;
        Ok(NoneType)
    }

    fn run_command(
        binding: &str,
        args: UnpackList<String>,
        eval: &mut Evaluator,
    ) -> anyhow::Result<NoneType> {
        command(config(eval)?, binding, &args.items)?;
        Ok(NoneType)
    }

    fn relative_path(root: &str, eval: &mut Evaluator) -> anyhow::Result<String> {
        let root = config(eval)?
            .roots
            .get(root)
            .with_context(|| format!("Missing workspace root '{root}'"))?
            .canonicalize()
            .context("Cannot resolve configured workspace root")?;
        let cwd = std::env::current_dir()?.canonicalize()?;
        let relative = cwd
            .strip_prefix(&root)
            .context("Current directory is outside the configured workspace root")?;
        Ok(relative.to_string_lossy().into_owned())
    }

    fn random_string(length: i32, alphabet: &str) -> anyhow::Result<String> {
        if !(0..=1_000_000).contains(&length) {
            bail!("Random string length must be between 0 and 1000000");
        }
        let chars: Vec<_> = alphabet.chars().collect();
        if chars.is_empty() {
            bail!("Random string alphabet must be nonempty");
        }
        let mut rng = OsRng;
        Ok((0..length)
            .map(|_| chars[rng.gen_range(0..chars.len())])
            .collect())
    }

    fn setting(name: &str, eval: &mut Evaluator) -> anyhow::Result<String> {
        config(eval)?
            .settings
            .get(name)
            .cloned()
            .with_context(|| format!("Missing setting '{name}'"))
    }
}
