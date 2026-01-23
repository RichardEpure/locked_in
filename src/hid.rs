use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

use super::config::Device;
use anyhow::{Context, Result};
use hidapi::{DeviceInfo, HidApi};

pub static HID_DEVICES: LazyLock<Mutex<HidDevices>> = LazyLock::new(|| {
    let mut devices = HidDevices::new();
    devices.refresh();
    Mutex::new(devices)
});

static HID_API: LazyLock<HidApi> =
    LazyLock::new(|| HidApi::new().expect("Failed to create HID API instance"));

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct HidMetadata {
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer_string: String,
    pub product_string: String,
    pub usages: HashSet<UsagePair>,
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct UsagePair {
    pub usage_page: u16,
    pub usage: u16,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct HidMetadataKey {
    pub vendor_id: u16,
    pub product_id: u16,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct HidDeviceKey {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
}

pub struct HidDevices {
    metadata_map: std::collections::HashMap<HidMetadataKey, HidMetadata>,
    device_info_map: std::collections::HashMap<HidDeviceKey, DeviceInfo>,
}

impl HidDevices {
    pub fn new() -> Self {
        HidDevices {
            metadata_map: std::collections::HashMap::new(),
            device_info_map: std::collections::HashMap::new(),
        }
    }

    pub fn refresh(&mut self) -> &mut Self {
        let mut metadata_map: std::collections::HashMap<HidMetadataKey, HidMetadata> =
            std::collections::HashMap::new();
        let mut device_info_map: std::collections::HashMap<HidDeviceKey, DeviceInfo> =
            std::collections::HashMap::new();

        for device_info in HID_API.device_list() {
            let metadata_key = HidMetadataKey {
                vendor_id: device_info.vendor_id(),
                product_id: device_info.product_id(),
            };
            let entry = metadata_map
                .entry(metadata_key)
                .or_insert_with(|| HidMetadata {
                    vendor_id: metadata_key.vendor_id,
                    product_id: metadata_key.product_id,
                    manufacturer_string: device_info
                        .manufacturer_string()
                        .unwrap_or_default()
                        .to_string(),
                    product_string: device_info.product_string().unwrap_or_default().to_string(),
                    usages: HashSet::new(),
                });
            entry.usages.insert(UsagePair {
                usage_page: device_info.usage_page(),
                usage: device_info.usage(),
            });

            let device_info_key = HidDeviceKey {
                vendor_id: device_info.vendor_id(),
                product_id: device_info.product_id(),
                usage_page: device_info.usage_page(),
                usage: device_info.usage(),
            };
            device_info_map.insert(device_info_key, device_info.clone());
        }

        self.metadata_map = metadata_map;
        self.device_info_map = device_info_map;
        self
    }

    pub fn get_metadata_list(&self) -> Vec<HidMetadata> {
        self.metadata_map.values().cloned().collect()
    }

    pub fn get(&self, key: &HidDeviceKey) -> Option<&DeviceInfo> {
        self.device_info_map.get(key)
    }
}

impl Device {
    pub fn send_report(&self, report: &[u8]) -> Result<usize> {
        let device_info = {
            let hid_devices = HID_DEVICES
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire HID_DEVICES lock"))?;

            hid_devices
                .get(&HidDeviceKey {
                    vendor_id: self.vid,
                    product_id: self.pid,
                    usage_page: self.usage_page,
                    usage: self.usage,
                })
                .cloned()
                .context("Device not found in cache")?
        };

        let hid_device = device_info
            .open_device(&HID_API)
            .context("Failed to open HID device")?;

        let report_length = self.report_length as usize;

        if report.len() > report_length {
            anyhow::bail!(
                "report length {} > expected {}",
                report.len(),
                report_length
            )
        }

        let mut bytes_to_write = vec![
            0u8;
            report_length
                .checked_add(1)
                .context("report_length too large (overflow)")?
        ];
        bytes_to_write[0] = self.report_id;
        let end = 1 + report.len();
        bytes_to_write[1..end].copy_from_slice(report);

        hid_device
            .write(&bytes_to_write)
            .with_context(|| "Failed to write to device")
    }
}
