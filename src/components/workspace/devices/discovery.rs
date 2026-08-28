use dioxus::{dioxus_core::spawn_forever, prelude::*};

use crate::{HID_CACHE_REVISION_SIGNAL, hid};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DiscoveryStatus {
    Idle,
    Refreshing,
    Ready(usize),
    Failed(String),
}

pub(super) static DISCOVERED_INTERFACES_SIGNAL: GlobalSignal<Vec<hid::DiscoveredInterface>> =
    Signal::global(Vec::new);
pub(super) static DISCOVERY_STATUS_SIGNAL: GlobalSignal<DiscoveryStatus> =
    Signal::global(|| DiscoveryStatus::Idle);

fn apply_discovery_result(
    discovered: &mut Vec<hid::DiscoveredInterface>,
    result: anyhow::Result<Vec<hid::DiscoveredInterface>>,
) -> DiscoveryStatus {
    match result {
        Ok(interfaces) => {
            let count = interfaces.len();
            *discovered = interfaces;
            DiscoveryStatus::Ready(count)
        }
        Err(error) => DiscoveryStatus::Failed(format!("{error:#}")),
    }
}

pub(super) fn refresh_discovered_interfaces() {
    if *DISCOVERY_STATUS_SIGNAL.peek() == DiscoveryStatus::Refreshing {
        return;
    }
    *DISCOVERY_STATUS_SIGNAL.write() = DiscoveryStatus::Refreshing;
    spawn_forever(async move {
        let result = match tokio::task::spawn_blocking(hid::discovered_interfaces).await {
            Ok(result) => result,
            Err(error) => Err(anyhow::anyhow!("Discovery task failed: {error}")),
        };
        let refreshed = result.is_ok();
        let next_status = apply_discovery_result(&mut DISCOVERED_INTERFACES_SIGNAL.write(), result);
        *DISCOVERY_STATUS_SIGNAL.write() = next_status;
        if refreshed {
            *HID_CACHE_REVISION_SIGNAL.write() += 1;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn discovered_interface(name: &str, usage: u16) -> hid::DiscoveredInterface {
        hid::DiscoveredInterface {
            name: name.into(),
            vendor_id: 1,
            product_id: 2,
            usage_page: 3,
            usage,
        }
    }

    #[test]
    fn successful_discovery_replaces_interfaces_even_when_count_is_unchanged() {
        let mut discovered = vec![discovered_interface("Old", 4)];

        let status = apply_discovery_result(
            &mut discovered,
            Ok(vec![discovered_interface("Current", 5)]),
        );

        assert_eq!(status, DiscoveryStatus::Ready(1));
        assert_eq!(discovered, [discovered_interface("Current", 5)]);
    }

    #[test]
    fn failed_discovery_retains_previously_displayed_interfaces() {
        let original = discovered_interface("Current", 5);
        let mut discovered = vec![original.clone()];

        let status = apply_discovery_result(
            &mut discovered,
            Err(anyhow::anyhow!("enumeration unavailable")),
        );

        assert_eq!(
            status,
            DiscoveryStatus::Failed("enumeration unavailable".into())
        );
        assert_eq!(discovered, [original]);
    }
}
