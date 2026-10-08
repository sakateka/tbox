use anyhow::{Context, Result, bail};
use serde::Deserialize;
use starlark::any::ProvidesStaticType;
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, Deserialize, ProvidesStaticType)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub aliases: BTreeMap<String, String>,
    pub applets: BTreeMap<String, Applet>,
    pub http: BTreeMap<String, HttpProfile>,
    pub links: BTreeMap<String, LinkProfile>,
    pub commands: BTreeMap<String, Vec<String>>,
    pub roots: BTreeMap<String, PathBuf>,
    pub settings: BTreeMap<String, String>,
    #[serde(skip)]
    pub source: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Applet {
    pub description: String,
    pub usage: String,
    pub script: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpProfile {
    pub url: String,
    #[serde(default = "get_method")]
    pub method: String,
    pub body: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    pub token_env: Option<String>,
    pub token_file: Option<PathBuf>,
    #[serde(default = "oauth_scheme")]
    pub auth_scheme: String,
    pub ca_file: Option<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}
fn get_method() -> String {
    "GET".into()
}
fn oauth_scheme() -> String {
    "OAuth".into()
}
fn default_timeout() -> u64 {
    30
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkProfile {
    pub url: String,
    pub command: String,
}

pub fn expand_path(path: &Path, base: &Path) -> Result<PathBuf> {
    let text = path.to_string_lossy();
    if text == "~" || text.starts_with("~/") {
        let home = env::var_os("HOME").context("HOME is required to expand ~")?;
        return Ok(PathBuf::from(home).join(text.strip_prefix("~/").unwrap_or("")));
    }
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    })
}

impl Config {
    pub fn load(explicit: Option<PathBuf>) -> Result<Self> {
        let selected = explicit.or_else(|| env::var_os("TBOX_CONFIG").map(PathBuf::from));
        let required = selected.is_some();
        let path = expand_path(
            &selected.unwrap_or_else(|| "~/.config/tbox/config.toml".into()),
            &env::current_dir()?,
        )?;
        let mut cfg: Self = match fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("Invalid config {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => Self::default(),
            Err(e) => {
                return Err(e).with_context(|| format!("Cannot read config {}", path.display()));
            }
        };
        let base = path.parent().context("Config has no parent directory")?;
        for (name, applet) in &cfg.applets {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
                || applet.description.trim().is_empty()
                || applet.usage.trim().is_empty()
                || applet.script.trim().is_empty()
            {
                bail!(
                    "Invalid applet '{name}': description, usage and script must be nonempty; names use letters, digits, _ or -"
                );
            }
        }
        for root in cfg.roots.values_mut() {
            *root = expand_path(root, base)?;
        }
        for argv in cfg.commands.values_mut() {
            if argv.is_empty() || argv[0].is_empty() {
                bail!("Command bindings require a nonempty executable");
            }
            if argv[0].contains('/') {
                argv[0] = expand_path(Path::new(&argv[0]), base)?
                    .to_string_lossy()
                    .into_owned();
            }
        }
        for profile in cfg.http.values_mut() {
            for path in [&mut profile.token_file, &mut profile.ca_file]
                .into_iter()
                .flatten()
            {
                *path = expand_path(path, base)?;
            }
        }
        cfg.source = path;
        Ok(cfg)
    }
}
