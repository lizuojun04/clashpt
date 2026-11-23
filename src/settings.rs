use crate::tui::theme::Theme;
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_yaml::{Mapping, Value};
use std::{env, fs, io, num::NonZeroUsize, path::{Path, PathBuf}};

const DEFAULT_FILE: &str = "clashpt.yaml";
const DEFAULT_SETTINGS: &str = include_str!("default_clashpt.yaml");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) theme: Theme,
    pub(crate) indicators: Indicators,
    pub(crate) latency_test: LatencyTest,
    #[serde(default)]
    pub(crate) system: System,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Indicators {
    pub(crate) active_node: String,
    active_profile: Option<String>,
    pub(crate) running: String,
    pub(crate) stopped: String,
}

impl Indicators {
    pub(crate) fn active_profile(&self) -> &str {
        self.active_profile.as_deref().unwrap_or(&self.running)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LatencyTest {
    pub(crate) concurrency: NonZeroUsize,
}

#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct System {
    pub(crate) gateway_workers: Option<NonZeroUsize>,
}

impl Default for Settings {
    fn default() -> Self {
        serde_yaml::from_str(DEFAULT_SETTINGS).expect("read built-in ClashPT settings")
    }
}

impl Settings {
    pub(crate) fn load(path: Option<&Path>) -> Result<Self> {
        let Some(file) = path.map(Path::to_path_buf).or_else(default_path) else {
            return Ok(Self::default());
        };
        let content = match fs::read_to_string(&file) {
            Ok(content) => content,
            Err(error) if path.is_none() && error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).with_context(|| format!("read {}", file.display())),
        };
        let overrides: Option<Mapping> = serde_yaml::from_str(&content)
            .with_context(|| format!("parse {}", file.display()))?;
        let mut defaults: Mapping = serde_yaml::from_str(DEFAULT_SETTINGS)
            .context("read built-in ClashPT settings")?;
        for (key, value) in overrides.unwrap_or_default() {
            match (defaults.get_mut(&key), value) {
                (Some(Value::Mapping(section)), Value::Mapping(overrides)) => section.extend(overrides),
                (_, value) => { defaults.insert(key, value); }
            }
        }

        let settings: Self = serde_yaml::from_value(Value::Mapping(defaults))
            .with_context(|| format!("parse {}", file.display()))?;
        for (key, symbol) in [
            ("active_node", settings.indicators.active_node.as_str()),
            ("active_profile", settings.indicators.active_profile()),
            ("running", settings.indicators.running.as_str()),
            ("stopped", settings.indicators.stopped.as_str()),
        ] {
            ensure!(!symbol.chars().any(char::is_control),
                "{}: indicators.{key} cannot contain control characters", file.display());
        }
        Ok(settings)
    }
}

fn default_path() -> Option<PathBuf> {
    env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").filter(|path| !path.is_empty())
            .map(|home| PathBuf::from(home).join(".config")))
        .map(|path| path.join("clashpt").join(DEFAULT_FILE))
}
