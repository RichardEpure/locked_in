use anyhow::{Context, Result};
use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

use crate::win::WindowMetadata;

const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Clone, Deserialize, strum_macros::EnumIter, strum_macros::Display)]
#[serde(tag = "type", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Event {
    FocusedWindowChanged(FocusedWindowChangedConfig),
}

impl Default for Event {
    fn default() -> Self {
        Self::FocusedWindowChanged(FocusedWindowChangedConfig::default())
    }
}

impl Serialize for Event {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Event::FocusedWindowChanged(cfg) => {
                map.serialize_entry("type", "focused_window_changed")?;
                map.serialize_entry("inclusions", &cfg.inclusions)?;
                map.serialize_entry("exclusions", &cfg.exclusions)?;
            }
        }
        map.end()
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct FocusedWindowChangedConfig {
    #[serde(default)]
    pub inclusions: Vec<WindowMetadata>,
    #[serde(default)]
    pub exclusions: Vec<WindowMetadata>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Device {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub report_length: usize,
    pub report_id: u8,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub name: String,
    pub event: Event,
    #[serde(default)]
    pub devices: Vec<Device>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Config {
    pub fn load() -> Self {
        let path = env::current_dir()
            .unwrap_or_else(|e| panic!("Failed to get current directory: {}", e))
            .join(CONFIG_PATH);

        if path.is_file() {
            return Figment::new()
                .merge(Toml::file(CONFIG_PATH))
                .extract::<Config>()
                .map_err(Box::new)
                .unwrap_or_else(|e| panic!("Failed to load config.toml: {}", e));
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).unwrap_or_else(|e| {
                panic!(
                    "Failed to create config directory: {}. {}",
                    parent.display(),
                    e
                )
            });
        }

        let _ = fs::OpenOptions::new().create_new(true);

        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        fn tmp_path_for(dest: &Path) -> PathBuf {
            let mut name = dest.file_name().unwrap_or_default().to_os_string();
            name.push(".tmp");

            let tmp_name = {
                // prefix with dot
                let mut s = std::ffi::OsString::from(".");
                s.push(name);
                s
            };

            dest.with_file_name(tmp_name)
        }

        let path = env::current_dir()
            .context("Failed to get current directory")?
            .join(CONFIG_PATH);

        let toml_string =
            toml::to_string_pretty(self).context("Failed to serialize config to TOML")?;

        let tmp_path = tmp_path_for(&path);

        {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)
                .with_context(|| {
                    format!("Failed to open temp config file: {}", tmp_path.display())
                })?;

            file.write_all(toml_string.as_bytes()).with_context(|| {
                format!("Failed to write config TOML to: {}", tmp_path.display())
            })?;

            file.flush()
                .with_context(|| format!("Failed to flush config file: {}", tmp_path.display()))?;
        }

        fs::rename(&tmp_path, &path).with_context(|| {
            format!(
                "Failed to replace config file (rename {} -> {})",
                tmp_path.display(),
                path.display(),
            )
        })?;

        Ok(())
    }

    pub fn delete_rule(&mut self, name: &str) {
        if let Some(index) = self.rules.iter().position(|r| r.name == name) {
            self.rules.remove(index);
        }
    }

    pub fn get_rule_index(&self, name: &str) -> Option<usize> {
        self.rules.iter().position(|r| r.name == name)
    }

    pub fn get_rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }
}
