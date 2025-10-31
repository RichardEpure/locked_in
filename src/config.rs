use figment::{
    Figment,
    providers::{Format, Toml},
};
use serde::Deserialize;
use std::{env, fs, io::Write};

const CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Deserialize)]
pub struct Device {
    pub name: String,
    pub vid: u16,
    pub pid: u16,
    pub usage_page: u16,
    pub usage: u16,
    pub report_length: usize,
    pub report_id: u8,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub devices: Vec<Device>,
}

fn config_to_string(config: &Config) -> String {
    let mut toml_string = r##"# Example entry for a device
#
# [[devices]]
# name = "My Device"
# vid = 0x1234
# pid = 0x5678
# usage_page = 0x1100
# usage = 0x0011
# report_length = 32
# report_id = 0x00
"##
    .to_string();

    for device in config.devices.iter() {
        let device_string = format!(
            r##"
[[devices]]
name = "{}"
vid = 0x{:04x}
pid = 0x{:04x}
usage_page = 0x{:04x}
usage = 0x{:04x}
report_length = {}
report_id = 0x{:04x}
        "##,
            device.name,
            device.vid,
            device.pid,
            device.usage_page,
            device.usage,
            device.report_length,
            device.report_id
        );
        toml_string.push_str(&device_string);
    }

    toml_string
}

pub fn init_config() -> Config {
    let path = env::current_dir()
        .unwrap_or_else(|e| panic!("Failed to get config path: {}", e))
        .join(CONFIG_PATH);

    if path.is_file() {
        return load_config();
    }

    const TEMPLATE: &str = r##"# Example entry for a device
#
# [[devices]]
# name = "My Device"
# vid = 0x1234
# pid = 0x5678
# usage_page = 0x1100
# usage = 0x0011
# report_length = 32
# report_id = 0x00
"##;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("Failed to create config.toml at {}: {}", CONFIG_PATH, e));

    file.write_all(TEMPLATE.as_bytes())
        .unwrap_or_else(|e| panic!("Failed to write config.toml to {}: {}", CONFIG_PATH, e));
    file.flush()
        .unwrap_or_else(|e| panic!("Failed to flush config.toml to {}: {}", CONFIG_PATH, e));

    load_config()
}

pub fn load_config() -> Config {
    Figment::new()
        .merge(Toml::file(CONFIG_PATH))
        .extract::<Config>()
        .map_err(Box::new)
        .unwrap_or_else(|e| panic!("Failed to load config.toml: {}", e))
}

pub fn save_config(config: &Config) {
    let path = env::current_dir().unwrap().join(CONFIG_PATH);
    let toml_string = config_to_string(config);

    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();

    file.write_all(toml_string.as_bytes())
        .unwrap_or_else(|e| panic!("Failed to write config.toml to {}: {}", CONFIG_PATH, e));
    file.flush()
        .unwrap_or_else(|e| panic!("Failed to flush config.toml to {}: {}", CONFIG_PATH, e));
}

pub fn append_device(config: &mut Config, device: Device) {
    match config.devices.iter().find(|d| d.name == device.name) {
        Some(_) => {
            println!(
                "Device {} already exists in config, skipping append.",
                device.name
            );
        }
        None => {
            config.devices.push(device);
        }
    }
}
