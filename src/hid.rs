use std::{collections::HashSet, sync::LazyLock};

use super::config::Device;
use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};

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

static HID_API: LazyLock<HidApi> =
    LazyLock::new(|| HidApi::new().expect("Failed to create HID API instance"));

pub fn find_device(device: &Device) -> Result<HidDevice> {
    for device_info in HID_API.device_list() {
        if device_info.vendor_id() == device.vid
            && device_info.product_id() == device.pid
            && device_info.usage_page() == device.usage_page
            && device_info.usage() == device.usage
        {
            return device_info
                .open_device(&HID_API)
                .with_context(|| "Failed to open device");
        }
    }

    anyhow::bail!("Device not found")
}

pub fn send_report(hid_device: &HidDevice, device: &Device, report: &[u8]) -> Result<usize> {
    if report.len() != device.report_length as usize {
        panic!(
            "report length {} != expected {}",
            report.len(),
            device.report_length
        )
    }

    let mut bytes_to_write = vec![0u8; device.report_length as usize + 1];
    bytes_to_write[0] = device.report_id;
    bytes_to_write[1..].copy_from_slice(report);

    hid_device
        .write(&bytes_to_write)
        .with_context(|| "Failed to write to device")
}

pub fn get_devices() -> Vec<HidMetadata> {
    let mut devices: Vec<HidMetadata> = vec![];
    let mut seen: HashSet<(u16, u16, String, String)> = HashSet::new();
    for device_info in HID_API.device_list() {
        let meta = HidMetadata {
            vendor_id: device_info.vendor_id(),
            product_id: device_info.product_id(),
            manufacturer_string: device_info
                .manufacturer_string()
                .unwrap_or_default()
                .to_string(),
            product_string: device_info.product_string().unwrap_or_default().to_string(),
            usages: HashSet::from([UsagePair {
                usage_page: device_info.usage_page(),
                usage: device_info.usage(),
            }]),
        };

        let key = (
            meta.vendor_id,
            meta.product_id,
            meta.manufacturer_string.clone(),
            meta.product_string.clone(),
        );

        if seen.insert(key) {
            devices.push(meta);
        } else {
            devices
                .iter_mut()
                .filter(|d| {
                    d.vendor_id == meta.vendor_id
                        && d.product_id == meta.product_id
                        && d.manufacturer_string == meta.manufacturer_string
                        && d.product_string == meta.product_string
                })
                .take(1)
                .for_each(|d| {
                    let _ = d.usages.insert(UsagePair {
                        usage_page: device_info.usage_page(),
                        usage: device_info.usage(),
                    });
                });
        }
    }
    devices
}
