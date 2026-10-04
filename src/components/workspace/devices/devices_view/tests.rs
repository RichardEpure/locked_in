use super::*;
use crate::hid::{HidError, HidInventoryRow, InterfaceSelector};

fn inventory() -> HidInventory {
    HidInventory {
        revision: 5,
        refresh_state: HidRefreshState::Ready,
        rows: vec![HidInventoryRow {
            selector: InterfaceSelector {
                vendor_id: 1,
                product_id: 2,
                usage_page: 3,
                usage: 4,
            },
            name: "Deck".into(),
            match_count: 1,
        }],
    }
}

#[test]
fn adoption_uses_current_inventory_and_creates_a_locked_unsaved_draft() {
    let inventory = inventory();
    let config = EditableConfig::default();
    let selector = inventory.rows[0].selector;
    let Some(DiscoverySelection::Create(draft)) =
        discovery_selection(DiscoveryIntent::Adopt(selector), &inventory, &config, 7)
    else {
        panic!("expected creation");
    };
    assert!(draft.is_new() && draft.is_dirty());
    assert_eq!(draft.edited.name, "Deck");
    assert_eq!(InterfaceSelector::from(&draft.edited), selector);
    assert_eq!(draft.edited.report_length, 32);
    let mut dom = VirtualDom::new(VNode::empty);
    dom.rebuild_in_place();
    dom.in_scope(ScopeId::ROOT, || {
        let pending = Signal::new(None);
        let selected = Signal::new(None);
        let id = draft.edited.id.clone();
        begin_device_draft(draft, pending, selected);
        assert_eq!(*selected.read(), Some(id.clone()));
        assert!(pending.read().as_ref().unwrap().is_new());
        // The editor has not mounted yet.
        assert_eq!(*DIRTY_EDITOR_SIGNAL.read(), Some(format!("device:{id}")));
    });
}

#[test]
fn stale_ambiguous_failed_and_missing_rows_reject_both_intents() {
    let mut inventory = inventory();
    let selector = inventory.rows[0].selector;
    let config = EditableConfig::default();
    for state in [
        HidRefreshState::NotAttempted,
        HidRefreshState::Refreshing,
        HidRefreshState::Failed {
            error: HidError::InventoryUnavailable,
        },
    ] {
        inventory.refresh_state = state;
        for intent in [
            DiscoveryIntent::Adopt(selector),
            DiscoveryIntent::OpenSaved(selector),
        ] {
            assert_eq!(discovery_selection(intent, &inventory, &config, 1), None);
        }
    }
    inventory.refresh_state = HidRefreshState::Ready;
    inventory.rows[0].match_count = 2;
    assert_eq!(
        discovery_selection(DiscoveryIntent::Adopt(selector), &inventory, &config, 1),
        None
    );
    inventory.rows.clear();
    assert_eq!(
        discovery_selection(DiscoveryIntent::Adopt(selector), &inventory, &config, 1),
        None
    );
}

#[test]
fn saved_aliases_are_rechecked_before_adoption_or_open() {
    let inventory = inventory();
    let selector = inventory.rows[0].selector;
    let mut config = EditableConfig::default();
    let Some(DiscoverySelection::Create(draft)) =
        discovery_selection(DiscoveryIntent::Adopt(selector), &inventory, &config, 1)
    else {
        panic!("expected creation");
    };
    let id = draft.edited.id.clone();
    config.devices.push(draft.edited.clone());
    assert_eq!(
        discovery_selection(DiscoveryIntent::Adopt(selector), &inventory, &config, 2),
        None
    );
    assert_eq!(
        discovery_selection(DiscoveryIntent::OpenSaved(selector), &inventory, &config, 2),
        Some(DiscoverySelection::Open(id))
    );
    config.devices.push(Device {
        id: "other-alias".into(),
        ..draft.edited
    });
    assert_eq!(
        discovery_selection(DiscoveryIntent::OpenSaved(selector), &inventory, &config, 3),
        None
    );
}
