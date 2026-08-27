use std::{
    collections::{HashMap, HashSet},
    sync::{LazyLock, Mutex, MutexGuard},
};

use super::config::Device;
use anyhow::{Context, Result};
use hidapi::{DeviceInfo, HidApi};

pub static HID_DEVICES: LazyLock<Mutex<HidDevices>> =
    LazyLock::new(|| Mutex::new(HidDevices::new()));

static HID_API: LazyLock<std::result::Result<Mutex<HidApi>, hidapi::HidError>> =
    LazyLock::new(|| HidApi::new().map(Mutex::new));
static HID_CACHE_INITIALIZATION: LazyLock<Mutex<CacheInitialization>> =
    LazyLock::new(|| Mutex::new(CacheInitialization::NotAttempted));

enum CacheInitialization {
    NotAttempted,
    Ready,
    Failed(String),
}

fn lock_hid_api() -> Result<MutexGuard<'static, HidApi>> {
    HID_API
        .as_ref()
        .map_err(|error| anyhow::anyhow!("Failed to create HID API instance: {error}"))?
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to acquire HID API lock"))
}

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
    metadata_map: HashMap<HidMetadataKey, HidMetadata>,
    device_info_map: HashMap<HidDeviceKey, DeviceInfo>,
}

impl HidDevices {
    pub fn new() -> Self {
        HidDevices {
            metadata_map: HashMap::new(),
            device_info_map: HashMap::new(),
        }
    }

    pub fn refresh(&mut self) -> Result<&mut Self> {
        let mut metadata_map: HashMap<HidMetadataKey, HidMetadata> = HashMap::new();
        let mut device_info_map: HashMap<HidDeviceKey, DeviceInfo> = HashMap::new();
        let mut api = lock_hid_api()?;
        api.refresh_devices()
            .context("Failed to refresh HID devices")?;

        for device_info in api.device_list() {
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
        Ok(self)
    }

    pub fn get_metadata_list(&self) -> Vec<HidMetadata> {
        self.metadata_map.values().cloned().collect()
    }

    pub fn get(&self, key: &HidDeviceKey) -> Option<&DeviceInfo> {
        self.device_info_map.get(key)
    }
}

fn replace_device_cache() -> Result<Vec<HidMetadata>> {
    let mut refreshed = HidDevices::new();
    refreshed.refresh()?;
    let metadata = refreshed.get_metadata_list();
    *HID_DEVICES
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to acquire HID device cache lock"))? = refreshed;
    Ok(metadata)
}

fn refresh_device_cache() -> Result<Vec<HidMetadata>> {
    let mut initialization = HID_CACHE_INITIALIZATION
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to acquire HID cache initialization lock"))?;
    match replace_device_cache() {
        Ok(metadata) => {
            *initialization = CacheInitialization::Ready;
            Ok(metadata)
        }
        Err(error) => {
            let message = format!("{error:#}");
            *initialization = CacheInitialization::Failed(message.clone());
            Err(anyhow::anyhow!(message))
        }
    }
}

pub fn initialize_device_cache() -> Result<()> {
    let mut initialization = HID_CACHE_INITIALIZATION
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to acquire HID cache initialization lock"))?;
    match &*initialization {
        CacheInitialization::Ready => return Ok(()),
        CacheInitialization::Failed(message) => return Err(anyhow::anyhow!(message.clone())),
        CacheInitialization::NotAttempted => {}
    }
    match replace_device_cache() {
        Ok(_) => {
            *initialization = CacheInitialization::Ready;
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            *initialization = CacheInitialization::Failed(message.clone());
            Err(anyhow::anyhow!(message))
        }
    }
}

impl Device {
    pub fn send_report(&self, report: &[u8]) -> Result<usize> {
        let key = HidDeviceKey {
            vendor_id: self.vid,
            product_id: self.pid,
            usage_page: self.usage_page,
            usage: self.usage,
        };
        let mut device_info = HID_DEVICES
            .lock()
            .map_err(|_| anyhow::anyhow!("Failed to acquire HID device cache lock"))?
            .get(&key)
            .cloned();
        if device_info.is_none() {
            initialize_device_cache()?;
            device_info = HID_DEVICES
                .lock()
                .map_err(|_| anyhow::anyhow!("Failed to acquire HID device cache lock"))?
                .get(&key)
                .cloned();
        }
        let device_info = device_info.context("Device not found in cache")?;

        let api = lock_hid_api()?;
        let hid_device = device_info
            .open_device(&api)
            .context("Failed to open HID device")?;

        let bytes_to_write = frame_report(self.report_id, self.report_length, report)?;

        hid_device
            .write(&bytes_to_write)
            .with_context(|| "Failed to write to device")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredInterface {
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
}

pub fn is_connected(device: &Device) -> bool {
    HID_DEVICES.lock().is_ok_and(|devices| {
        devices
            .get(&HidDeviceKey {
                vendor_id: device.vid,
                product_id: device.pid,
                usage_page: device.usage_page,
                usage: device.usage,
            })
            .is_some()
    })
}

pub fn discovered_interfaces() -> Result<Vec<DiscoveredInterface>> {
    let mut interfaces = refresh_device_cache()?
        .into_iter()
        .flat_map(|metadata| {
            let name = if metadata.product_string.is_empty() {
                if metadata.manufacturer_string.is_empty() {
                    format!("{:04X}:{:04X}", metadata.vendor_id, metadata.product_id)
                } else {
                    metadata.manufacturer_string.clone()
                }
            } else {
                metadata.product_string.clone()
            };
            metadata
                .usages
                .into_iter()
                .map(move |usage| DiscoveredInterface {
                    name: name.clone(),
                    vendor_id: metadata.vendor_id,
                    product_id: metadata.product_id,
                    usage_page: usage.usage_page,
                    usage: usage.usage,
                })
        })
        .collect::<Vec<_>>();
    interfaces.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.vendor_id.cmp(&right.vendor_id))
            .then(left.product_id.cmp(&right.product_id))
            .then(left.usage_page.cmp(&right.usage_page))
            .then(left.usage.cmp(&right.usage))
    });
    Ok(interfaces)
}

fn frame_report(report_id: u8, report_length: u16, report: &[u8]) -> Result<Vec<u8>> {
    let report_length = report_length as usize;
    if report.len() > report_length {
        anyhow::bail!(
            "report length {} > expected {}",
            report.len(),
            report_length
        )
    }
    let mut bytes = vec![0; report_length + 1];
    bytes[0] = report_id;
    bytes[1..=report.len()].copy_from_slice(report);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_prefixed_and_padded() {
        let bytes = frame_report(7, 4, &[1, 2]).unwrap();
        assert_eq!(bytes, [7, 1, 2, 0, 0]);
    }

    #[test]
    fn oversized_report_is_rejected() {
        assert!(frame_report(0, 1, &[1, 2]).is_err());
    }
}
