mod system;

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{self, Display, Formatter},
    hash::Hash,
    time::{Duration, Instant},
};

use crate::config::Device;

const IMPLICIT_REFRESH_COOLDOWN: Duration = Duration::from_secs(1);

pub(crate) trait HidBackend: Send + 'static {
    fn inventory(&self) -> HidInventory;
    fn refresh(&mut self) -> HidInventory;
    fn send_report(&mut self, device: &Device, report: &[u8]) -> Result<(), HidError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct InterfaceSelector {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
}

impl From<&Device> for InterfaceSelector {
    fn from(device: &Device) -> Self {
        Self {
            vendor_id: device.vid,
            product_id: device.pid,
            usage_page: device.usage_page,
            usage: device.usage,
        }
    }
}

impl Display for InterfaceSelector {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04X}:{:04X} usage page {:04X} usage {:04X}",
            self.vendor_id, self.product_id, self.usage_page, self.usage
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HidError {
    InvalidReportLength,
    ReportTooLong {
        payload_length: usize,
        capacity: usize,
    },
    Disconnected {
        selector: InterfaceSelector,
    },
    Ambiguous {
        selector: InterfaceSelector,
        matches: usize,
    },
    Initialization {
        message: String,
    },
    Enumeration {
        message: String,
    },
    InventoryUnavailable,
    ResolutionInvalidated {
        selector: InterfaceSelector,
    },
    Open {
        selector: InterfaceSelector,
        message: String,
    },
    Write {
        selector: InterfaceSelector,
        message: String,
    },
    ShortWrite {
        selector: InterfaceSelector,
        expected: usize,
        actual: usize,
    },
}

impl Display for HidError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReportLength => {
                write!(formatter, "HID report length must be greater than zero")
            }
            Self::ReportTooLong {
                payload_length,
                capacity,
            } => write!(
                formatter,
                "HID report payload is {payload_length} bytes; configured capacity is {capacity} bytes"
            ),
            Self::Disconnected { selector } => {
                write!(formatter, "HID interface {selector} is disconnected")
            }
            Self::Ambiguous { selector, matches } => write!(
                formatter,
                "HID interface {selector} is ambiguous ({matches} matches)"
            ),
            Self::Initialization { message } => {
                write!(formatter, "HID initialization failed: {message}")
            }
            Self::Enumeration { message } => {
                write!(formatter, "HID enumeration failed: {message}")
            }
            Self::InventoryUnavailable => write!(formatter, "HID inventory is unavailable"),
            Self::ResolutionInvalidated { selector } => write!(
                formatter,
                "HID interface {selector} requires a refresh after an I/O failure"
            ),
            Self::Open { selector, message } => {
                write!(
                    formatter,
                    "failed to open HID interface {selector}: {message}"
                )
            }
            Self::Write { selector, message } => {
                write!(
                    formatter,
                    "failed to write HID interface {selector}: {message}"
                )
            }
            Self::ShortWrite {
                selector,
                expected,
                actual,
            } => write!(
                formatter,
                "short write to HID interface {selector}: wrote {actual} of {expected} bytes"
            ),
        }
    }
}

impl Error for HidError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HidRefreshState {
    NotAttempted,
    Refreshing,
    Ready,
    Failed { error: HidError },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HidInventoryRow {
    pub selector: InterfaceSelector,
    pub name: String,
    pub match_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HidInventory {
    pub revision: u64,
    pub refresh_state: HidRefreshState,
    pub rows: Vec<HidInventoryRow>,
}

impl Default for HidInventory {
    fn default() -> Self {
        Self {
            revision: 0,
            refresh_state: HidRefreshState::NotAttempted,
            rows: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HidPresence {
    Connected,
    Disconnected,
    Ambiguous { matches: usize },
    Unknown,
}

impl HidInventory {
    pub fn presence(&self, device: &Device) -> HidPresence {
        if self.refresh_state != HidRefreshState::Ready {
            return HidPresence::Unknown;
        }

        let selector = InterfaceSelector::from(device);
        match self.rows.iter().find(|row| row.selector == selector) {
            None => HidPresence::Disconnected,
            Some(row) if row.match_count == 1 => HidPresence::Connected,
            Some(row) => HidPresence::Ambiguous {
                matches: row.match_count,
            },
        }
    }
}

#[derive(Debug, Clone)]
struct ObservedInterface<Locator> {
    selector: InterfaceSelector,
    manufacturer_name: String,
    product_name: String,
    locator: Locator,
}

trait HidIo {
    type Locator: Clone;
    type Handle;

    fn enumerate(&mut self) -> Result<Vec<ObservedInterface<Self::Locator>>, HidError>;
    fn open(&mut self, locator: &Self::Locator) -> Result<Self::Handle, String>;
    fn write(&mut self, handle: &mut Self::Handle, report: &[u8]) -> Result<usize, String>;
}

trait Clock {
    fn now(&self) -> Duration;
}

struct SystemClock {
    started_at: Instant,
}

impl SystemClock {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.started_at.elapsed()
    }
}

struct HidCore<I: HidIo, C: Clock> {
    io: I,
    clock: C,
    inventory: HidInventory,
    locators: HashMap<InterfaceSelector, Vec<I::Locator>>,
    invalidated: HashSet<InterfaceSelector>,
    last_refresh_attempt: Option<Duration>,
}

impl<I: HidIo, C: Clock> HidCore<I, C> {
    fn new(io: I, clock: C) -> Self {
        Self {
            io,
            clock,
            inventory: HidInventory::default(),
            locators: HashMap::new(),
            invalidated: HashSet::new(),
            last_refresh_attempt: None,
        }
    }

    fn inventory(&self) -> HidInventory {
        self.inventory.clone()
    }

    fn refresh(&mut self) -> HidInventory {
        self.refresh_internal();
        self.inventory()
    }

    fn refresh_internal(&mut self) {
        self.last_refresh_attempt = Some(self.clock.now());
        self.inventory.refresh_state = HidRefreshState::Refreshing;

        match self.io.enumerate() {
            Ok(observed) => {
                let (rows, locators) = group_observed(observed);
                self.inventory.rows = rows;
                self.locators = locators;
                self.invalidated.clear();
                self.inventory.refresh_state = HidRefreshState::Ready;
            }
            Err(error) => {
                self.locators.clear();
                self.inventory.refresh_state = HidRefreshState::Failed { error };
            }
        }
        self.inventory.revision = self.inventory.revision.wrapping_add(1);
    }

    fn send_report(&mut self, device: &Device, payload: &[u8]) -> Result<(), HidError> {
        let framed = frame_report(device.report_id, device.report_length, payload)?;
        let selector = InterfaceSelector::from(device);
        let mut refreshed = false;

        let mut locator = match self.resolve(selector) {
            Ok(locator) => locator,
            Err(_) if self.implicit_refresh_is_eligible() => {
                self.refresh_internal();
                refreshed = true;
                self.resolve(selector)?
            }
            Err(error) => return Err(error),
        };

        let mut handle = match self.io.open(&locator) {
            Ok(handle) => handle,
            Err(_) if !refreshed => {
                self.locators.remove(&selector);
                self.invalidated.insert(selector);
                self.refresh_internal();
                refreshed = true;
                locator = self.resolve(selector)?;
                match self.io.open(&locator) {
                    Ok(handle) => handle,
                    Err(message) => {
                        self.locators.remove(&selector);
                        self.invalidated.insert(selector);
                        return Err(HidError::Open { selector, message });
                    }
                }
            }
            Err(message) => {
                self.locators.remove(&selector);
                self.invalidated.insert(selector);
                return Err(HidError::Open { selector, message });
            }
        };

        debug_assert!(refreshed || self.locators.contains_key(&selector));
        let written = match self.io.write(&mut handle, &framed) {
            Ok(written) => written,
            Err(message) => {
                self.locators.remove(&selector);
                self.invalidated.insert(selector);
                return Err(HidError::Write { selector, message });
            }
        };
        if written != framed.len() {
            self.locators.remove(&selector);
            self.invalidated.insert(selector);
            return Err(HidError::ShortWrite {
                selector,
                expected: framed.len(),
                actual: written,
            });
        }
        Ok(())
    }

    fn resolve(&self, selector: InterfaceSelector) -> Result<I::Locator, HidError> {
        match &self.inventory.refresh_state {
            HidRefreshState::Failed { error } => return Err(error.clone()),
            HidRefreshState::NotAttempted | HidRefreshState::Refreshing => {
                return Err(HidError::InventoryUnavailable);
            }
            HidRefreshState::Ready => {}
        }
        if self.invalidated.contains(&selector) {
            return Err(HidError::ResolutionInvalidated { selector });
        }
        match self.locators.get(&selector).map(Vec::as_slice) {
            Some([locator]) => Ok(locator.clone()),
            Some(locators) if locators.len() > 1 => Err(HidError::Ambiguous {
                selector,
                matches: locators.len(),
            }),
            _ => Err(HidError::Disconnected { selector }),
        }
    }

    fn implicit_refresh_is_eligible(&self) -> bool {
        self.last_refresh_attempt.is_none_or(|last_attempt| {
            self.clock.now().saturating_sub(last_attempt) >= IMPLICIT_REFRESH_COOLDOWN
        })
    }
}

fn group_observed<Locator: Clone>(
    observed: Vec<ObservedInterface<Locator>>,
) -> (
    Vec<HidInventoryRow>,
    HashMap<InterfaceSelector, Vec<Locator>>,
) {
    let mut grouped: HashMap<InterfaceSelector, Vec<ObservedInterface<Locator>>> = HashMap::new();
    for interface in observed {
        grouped
            .entry(interface.selector)
            .or_default()
            .push(interface);
    }

    let mut rows = Vec::with_capacity(grouped.len());
    let mut locators = HashMap::with_capacity(grouped.len());
    for (selector, interfaces) in grouped {
        let fallback = format!("{:04X}:{:04X}", selector.vendor_id, selector.product_id);
        let mut names = interfaces
            .iter()
            .map(|interface| display_name(interface, &fallback));
        let first_name = names.next().unwrap_or_else(|| fallback.clone());
        let name = if names.all(|name| name == first_name) {
            first_name
        } else {
            fallback
        };
        rows.push(HidInventoryRow {
            selector,
            name,
            match_count: interfaces.len(),
        });
        locators.insert(
            selector,
            interfaces
                .into_iter()
                .map(|interface| interface.locator)
                .collect(),
        );
    }
    rows.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.selector.cmp(&right.selector))
    });
    (rows, locators)
}

fn display_name<Locator>(interface: &ObservedInterface<Locator>, fallback: &str) -> String {
    let product = interface.product_name.trim();
    if !product.is_empty() {
        return product.to_string();
    }
    let manufacturer = interface.manufacturer_name.trim();
    if !manufacturer.is_empty() {
        return manufacturer.to_string();
    }
    fallback.to_string()
}

fn frame_report(report_id: u8, report_length: u16, payload: &[u8]) -> Result<Vec<u8>, HidError> {
    let capacity = report_length as usize;
    if capacity == 0 {
        return Err(HidError::InvalidReportLength);
    }
    if payload.len() > capacity {
        return Err(HidError::ReportTooLong {
            payload_length: payload.len(),
            capacity,
        });
    }
    let mut framed = vec![0; capacity + 1];
    framed[0] = report_id;
    framed[1..1 + payload.len()].copy_from_slice(payload);
    Ok(framed)
}

pub(crate) struct SystemHidBackend {
    core: HidCore<system::SystemHidIo, SystemClock>,
}

impl SystemHidBackend {
    pub fn new() -> Self {
        Self {
            core: HidCore::new(system::SystemHidIo::new(), SystemClock::new()),
        }
    }
}

impl HidBackend for SystemHidBackend {
    fn inventory(&self) -> HidInventory {
        self.core.inventory()
    }

    fn refresh(&mut self) -> HidInventory {
        self.core.refresh()
    }

    fn send_report(&mut self, device: &Device, report: &[u8]) -> Result<(), HidError> {
        self.core.send_report(device, report)
    }
}

#[cfg(test)]
mod tests;
