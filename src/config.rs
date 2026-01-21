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

use crate::win::{FOCUSED_WINDOW, WindowMetadata};

const CONFIG_PATH: &str = "config.toml";

#[derive(
    Debug,
    Clone,
    Deserialize,
    strum_macros::EnumIter,
    strum_macros::EnumString,
    strum_macros::Display,
)]
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
                map.serialize_entry("on_match_reports", &cfg.on_match_reports)?;
                map.serialize_entry("on_no_match_reports", &cfg.on_no_match_reports)?;
            }
        }
        map.end()
    }
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct FocusedWindowChangedConfig {
    pub inclusions: Vec<WindowMetadata>,
    pub exclusions: Vec<WindowMetadata>,
    pub on_match_reports: Vec<Vec<u8>>,
    pub on_no_match_reports: Vec<Vec<u8>>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Device {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub report_length: u16,
    pub report_id: u8,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub name: String,
    pub event: Event,
    pub devices: Vec<Device>,
}

impl Rule {
    pub fn trigger(&self) {
        match &self.event {
            Event::FocusedWindowChanged(event_cfg) => {
                let Ok(window) = FOCUSED_WINDOW.read() else {
                    return;
                };

                let exclusion_found = event_cfg
                    .exclusions
                    .iter()
                    .any(|exclusion| window.match_any(exclusion));

                let inclusion_found = event_cfg
                    .inclusions
                    .iter()
                    .any(|inclusion| window.match_any(inclusion));

                let reports = if exclusion_found || !inclusion_found {
                    &event_cfg.on_no_match_reports
                } else {
                    &event_cfg.on_match_reports
                };

                for device in &self.devices {
                    for report in reports {
                        if let Err(e) = device.send_report(report) {
                            eprintln!("Failed to send report to device {}: {}", device.name, e);
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub rules: Vec<Rule>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = env::current_dir()
            .context("Failed to get current directory")?
            .join(CONFIG_PATH);

        if path.is_file() {
            return Figment::new()
                .merge(Toml::file(CONFIG_PATH))
                .extract::<Config>()
                .context("Failed to load config.toml");
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory: {}", parent.display())
            })?;
        }

        let _ = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .context("Failed to create new config file")?;

        Ok(Self::default())
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

    pub fn get_mut_rule(&mut self, name: &str) -> Option<&mut Rule> {
        self.rules.iter_mut().find(|r| r.name == name)
    }
}
