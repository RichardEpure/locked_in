use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

use super::model::{Automation, Device, EditableConfig, Settings};

const SCHEMA_VERSION: u8 = 2;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SchemaV2 {
    version: u8,
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    devices: Vec<Device>,
    #[serde(default)]
    automations: Vec<Automation>,
}

#[derive(Serialize)]
struct SchemaV2Ref<'a> {
    version: u8,
    settings: &'a Settings,
    devices: &'a [Device],
    automations: &'a [Automation],
}

impl Serialize for EditableConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SchemaV2Ref {
            version: SCHEMA_VERSION,
            settings: &self.settings,
            devices: &self.devices,
            automations: &self.automations,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EditableConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = SchemaV2::deserialize(deserializer)?;
        if encoded.version != SCHEMA_VERSION {
            return Err(D::Error::custom(format_args!(
                "unsupported config version {}; expected {SCHEMA_VERSION}",
                encoded.version
            )));
        }
        Ok(Self {
            settings: encoded.settings,
            devices: encoded.devices,
            automations: encoded.automations,
        })
    }
}

pub(super) fn encode(config: &EditableConfig) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(config)
}

pub(super) fn decode(contents: &str) -> Result<EditableConfig, toml::de::Error> {
    toml::from_str(contents)
}

#[cfg(test)]
mod tests;
