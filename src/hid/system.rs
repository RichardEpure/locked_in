use hidapi::{DeviceInfo, HidApi, HidDevice};

use super::{HidError, HidIo, InterfaceSelector, ObservedInterface};

pub(super) struct SystemHidIo {
    api: Option<HidApi>,
}

impl SystemHidIo {
    pub(super) fn new() -> Self {
        Self { api: None }
    }
}

impl HidIo for SystemHidIo {
    type Locator = DeviceInfo;
    type Handle = HidDevice;

    fn enumerate(&mut self) -> Result<Vec<ObservedInterface<Self::Locator>>, HidError> {
        if let Some(api) = self.api.as_mut() {
            if let Err(error) = api.refresh_devices() {
                self.api = None;
                return Err(HidError::Enumeration {
                    message: error.to_string(),
                });
            }
        } else {
            self.api = Some(HidApi::new().map_err(|error| match error {
                hidapi::HidError::InitializationError => HidError::Initialization {
                    message: error.to_string(),
                },
                _ => HidError::Enumeration {
                    message: error.to_string(),
                },
            })?);
        }

        let api = self.api.as_ref().expect("HID API was initialized");
        Ok(api
            .device_list()
            .map(|device| ObservedInterface {
                selector: InterfaceSelector {
                    vendor_id: device.vendor_id(),
                    product_id: device.product_id(),
                    usage_page: device.usage_page(),
                    usage: device.usage(),
                },
                manufacturer_name: device.manufacturer_string().unwrap_or_default().to_string(),
                product_name: device.product_string().unwrap_or_default().to_string(),
                locator: device.clone(),
            })
            .collect())
    }

    fn open(&mut self, locator: &Self::Locator) -> Result<Self::Handle, String> {
        let api = self
            .api
            .as_ref()
            .ok_or_else(|| "HID API is unavailable".to_string())?;
        locator.open_device(api).map_err(|error| error.to_string())
    }

    fn write(&mut self, handle: &mut Self::Handle, report: &[u8]) -> Result<usize, String> {
        handle.write(report).map_err(|error| error.to_string())
    }
}
