use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
};

use super::config::Device;
use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};

pub static HID_DEVICES: LazyLock<Mutex<HidDevices>> =
    LazyLock::new(|| Mutex::new(HidDevices::new()));

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
pub struct HidDeviceKey {
    pub vendor_id: u16,
    pub product_id: u16,
}

pub struct HidDevices {
    pub devices: std::collections::HashMap<HidDeviceKey, HidMetadata>,
}

impl HidDevices {
    pub fn new() -> Self {
        HidDevices {
            devices: std::collections::HashMap::new(),
        }
    }

    pub fn refresh(&mut self) -> &mut Self {
        let mut devices_map: std::collections::HashMap<HidDeviceKey, HidMetadata> =
            std::collections::HashMap::new();

        for device_info in HID_API.device_list() {
            let key = HidDeviceKey {
                vendor_id: device_info.vendor_id(),
                product_id: device_info.product_id(),
            };

            let entry = devices_map.entry(key).or_insert_with(|| HidMetadata {
                vendor_id: key.vendor_id,
                product_id: key.product_id,
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
        }

        self.devices = devices_map;
        self
    }

    pub fn get_list(&self) -> Vec<HidMetadata> {
        self.devices.values().cloned().collect()
    }
}

impl Device {
    pub fn find_hid_device(&self) -> Result<HidDevice> {
        for device_info in HID_API.device_list() {
            if device_info.vendor_id() == self.vid
                && device_info.product_id() == self.pid
                && device_info.usage_page() == self.usage_page
                && device_info.usage() == self.usage
            {
                return device_info
                    .open_device(&HID_API)
                    .with_context(|| "Failed to open device");
            }
        }
        anyhow::bail!("Device not found")
    }

    pub fn send_report(&self, report: &[u8]) -> Result<usize> {
        let hid_device = self.find_hid_device()?;
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
