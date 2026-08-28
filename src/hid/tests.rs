use std::{cell::Cell, collections::VecDeque, rc::Rc};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Operation {
    Enumerate,
    Open(u8),
    Write(u8, Vec<u8>),
}

struct FakeIo {
    enumerations: VecDeque<Result<Vec<ObservedInterface<u8>>, HidError>>,
    opens: VecDeque<Result<(), String>>,
    writes: VecDeque<Result<usize, String>>,
    operations: Vec<Operation>,
}

impl FakeIo {
    fn new(enumerations: impl IntoIterator<Item = Vec<ObservedInterface<u8>>>) -> Self {
        Self {
            enumerations: enumerations.into_iter().map(Ok).collect(),
            opens: VecDeque::new(),
            writes: VecDeque::new(),
            operations: Vec::new(),
        }
    }
}

impl HidIo for FakeIo {
    type Locator = u8;
    type Handle = u8;

    fn enumerate(&mut self) -> Result<Vec<ObservedInterface<Self::Locator>>, HidError> {
        self.operations.push(Operation::Enumerate);
        self.enumerations.pop_front().unwrap_or(Ok(Vec::new()))
    }

    fn open(&mut self, locator: &Self::Locator) -> Result<Self::Handle, String> {
        self.operations.push(Operation::Open(*locator));
        self.opens.pop_front().unwrap_or(Ok(())).map(|()| *locator)
    }

    fn write(&mut self, handle: &mut Self::Handle, report: &[u8]) -> Result<usize, String> {
        self.operations
            .push(Operation::Write(*handle, report.to_vec()));
        self.writes.pop_front().unwrap_or(Ok(report.len()))
    }
}

#[derive(Clone)]
struct FakeClock(Rc<Cell<Duration>>);

impl FakeClock {
    fn new() -> Self {
        Self(Rc::new(Cell::new(Duration::ZERO)))
    }

    fn advance(&self, duration: Duration) {
        self.0.set(self.0.get() + duration);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

fn selector(value: u16) -> InterfaceSelector {
    InterfaceSelector {
        vendor_id: value,
        product_id: value + 1,
        usage_page: value + 2,
        usage: value + 3,
    }
}

fn observed(selector: InterfaceSelector, locator: u8, product: &str) -> ObservedInterface<u8> {
    ObservedInterface {
        selector,
        manufacturer_name: String::new(),
        product_name: product.to_string(),
        locator,
    }
}

fn device(selector: InterfaceSelector) -> Device {
    Device {
        vid: selector.vendor_id,
        pid: selector.product_id,
        usage_page: selector.usage_page,
        usage: selector.usage,
        report_length: 4,
        report_id: 7,
        ..Device::default()
    }
}

fn core(
    enumerations: impl IntoIterator<Item = Vec<ObservedInterface<u8>>>,
) -> HidCore<FakeIo, FakeClock> {
    HidCore::new(FakeIo::new(enumerations), FakeClock::new())
}

#[test]
fn report_is_prefixed_with_configured_id_and_padded_to_payload_capacity() {
    assert_eq!(frame_report(7, 4, &[1, 2]).unwrap(), [7, 1, 2, 0, 0]);
    assert_eq!(frame_report(7, 4, &[]).unwrap(), [7, 0, 0, 0, 0]);
}

#[test]
fn zero_report_length_is_rejected_before_io() {
    let mut core = core([vec![observed(selector(1), 1, "Device")]]);
    let mut target = device(selector(1));
    target.report_length = 0;

    assert_eq!(
        core.send_report(&target, &[1]),
        Err(HidError::InvalidReportLength)
    );
    assert!(core.io.operations.is_empty());
}

#[test]
fn oversized_report_is_rejected_before_io() {
    let mut core = core([vec![observed(selector(1), 1, "Device")]]);
    let mut target = device(selector(1));
    target.report_length = 1;

    let error = core.send_report(&target, &[1, 2]).unwrap_err();

    assert!(matches!(error, HidError::ReportTooLong { .. }));
    assert!(core.io.operations.is_empty());
}

#[test]
fn refresh_groups_every_locator_and_sorts_rows_deterministically() {
    let first = selector(10);
    let duplicate = selector(20);
    let unnamed = selector(30);
    let mut core = core([vec![
        observed(duplicate, 2, "Conflicting A"),
        observed(unnamed, 4, ""),
        observed(first, 1, "Alpha"),
        observed(duplicate, 3, "Conflicting B"),
    ]]);

    let inventory = core.refresh();

    assert_eq!(inventory.rows[0].name, "0014:0015");
    assert_eq!(inventory.rows[0].match_count, 2);
    assert_eq!(inventory.rows[1].name, "001E:001F");
    assert_eq!(inventory.rows[2].name, "Alpha");
    assert_eq!(core.locators[&duplicate], [2, 3]);
}

#[test]
fn missing_selector_refreshes_once_during_cooldown_then_reports_disconnected() {
    let mut core = core([vec![]]);
    let target = device(selector(1));

    assert!(matches!(
        core.send_report(&target, &[1]),
        Err(HidError::Disconnected { .. })
    ));
    assert!(matches!(
        core.send_report(&target, &[1]),
        Err(HidError::Disconnected { .. })
    ));
    assert_eq!(core.io.operations, [Operation::Enumerate]);
}

#[test]
fn unique_selector_opens_and_writes_one_complete_framed_report() {
    let selected = selector(1);
    let mut core = core([vec![observed(selected, 9, "Device")]]);
    core.refresh();

    core.send_report(&device(selected), &[1, 2]).unwrap();

    assert_eq!(
        core.io.operations,
        [
            Operation::Enumerate,
            Operation::Open(9),
            Operation::Write(9, vec![7, 1, 2, 0, 0]),
        ]
    );
}

#[test]
fn duplicate_selector_is_ambiguous_without_open_or_write() {
    let selected = selector(1);
    let mut core = core([vec![
        observed(selected, 4, "Same"),
        observed(selected, 5, "Same"),
    ]]);
    core.refresh();

    let error = core.send_report(&device(selected), &[1]).unwrap_err();

    assert_eq!(
        error,
        HidError::Ambiguous {
            selector: selected,
            matches: 2,
        }
    );
    assert_eq!(core.io.operations, [Operation::Enumerate]);
}

#[test]
fn construction_and_enumeration_failures_can_recover_on_later_refreshes() {
    let selected = selector(1);
    let mut io = FakeIo::new([]);
    io.enumerations = VecDeque::from([
        Err(HidError::Initialization {
            message: "init".into(),
        }),
        Err(HidError::Enumeration {
            message: "scan".into(),
        }),
        Ok(vec![observed(selected, 1, "Recovered")]),
    ]);
    let mut core = HidCore::new(io, FakeClock::new());

    let initialization_failure = core.refresh();
    assert_eq!(initialization_failure.revision, 1);
    assert!(matches!(
        initialization_failure.refresh_state,
        HidRefreshState::Failed {
            error: HidError::Initialization { .. }
        }
    ));
    let enumeration_failure = core.refresh();
    assert_eq!(enumeration_failure.revision, 2);
    assert!(matches!(
        enumeration_failure.refresh_state,
        HidRefreshState::Failed {
            error: HidError::Enumeration { .. }
        }
    ));
    let recovered = core.refresh();
    assert_eq!(recovered.revision, 3);
    assert_eq!(recovered.refresh_state, HidRefreshState::Ready);
}

#[test]
fn cache_miss_recovers_newly_attached_interface_after_cooldown() {
    let selected = selector(1);
    let clock = FakeClock::new();
    let mut core = HidCore::new(
        FakeIo::new([vec![], vec![observed(selected, 8, "Attached")]]),
        clock.clone(),
    );
    core.refresh();
    clock.advance(IMPLICIT_REFRESH_COOLDOWN);

    core.send_report(&device(selected), &[1]).unwrap();

    assert_eq!(
        core.io.operations,
        [
            Operation::Enumerate,
            Operation::Enumerate,
            Operation::Open(8),
            Operation::Write(8, vec![7, 1, 0, 0, 0]),
        ]
    );
}

#[test]
fn stale_open_refreshes_and_retries_open_once() {
    let selected = selector(1);
    let mut core = core([
        vec![observed(selected, 1, "Device")],
        vec![observed(selected, 2, "Device")],
    ]);
    core.io.opens = VecDeque::from([Err("stale".into()), Ok(())]);
    core.refresh();

    core.send_report(&device(selected), &[1]).unwrap();

    assert_eq!(
        core.io.operations,
        [
            Operation::Enumerate,
            Operation::Open(1),
            Operation::Enumerate,
            Operation::Open(2),
            Operation::Write(2, vec![7, 1, 0, 0, 0]),
        ]
    );
}

#[test]
fn stale_open_refresh_fails_closed_if_selector_becomes_ambiguous() {
    let selected = selector(1);
    let mut core = core([
        vec![observed(selected, 1, "Device")],
        vec![
            observed(selected, 2, "Device"),
            observed(selected, 3, "Device"),
        ],
    ]);
    core.io.opens.push_back(Err("stale".into()));
    core.refresh();

    let error = core.send_report(&device(selected), &[1]).unwrap_err();

    assert_eq!(
        error,
        HidError::Ambiguous {
            selector: selected,
            matches: 2,
        }
    );
    assert_eq!(
        core.io.operations,
        [
            Operation::Enumerate,
            Operation::Open(1),
            Operation::Enumerate,
        ]
    );
}

#[test]
fn stale_open_reports_refresh_failure_instead_of_private_invalidation() {
    let selected = selector(1);
    let mut io = FakeIo::new([vec![observed(selected, 1, "Device")]]);
    io.enumerations.push_back(Err(HidError::Enumeration {
        message: "refresh unavailable".into(),
    }));
    io.opens.push_back(Err("stale".into()));
    let mut core = HidCore::new(io, FakeClock::new());
    core.refresh();

    assert!(matches!(
        core.send_report(&device(selected), &[1]),
        Err(HidError::Enumeration { .. })
    ));
}

#[test]
fn refreshed_open_failure_is_not_retried_again() {
    let selected = selector(1);
    let mut core = core([
        vec![observed(selected, 1, "Device")],
        vec![observed(selected, 2, "Device")],
    ]);
    core.io.opens = VecDeque::from([Err("stale".into()), Err("still stale".into())]);
    core.refresh();

    assert!(matches!(
        core.send_report(&device(selected), &[1]),
        Err(HidError::Open { .. })
    ));
    assert_eq!(
        core.io.operations,
        [
            Operation::Enumerate,
            Operation::Open(1),
            Operation::Enumerate,
            Operation::Open(2),
        ]
    );
    assert!(!core.locators.contains_key(&selected));
}

#[test]
fn write_error_is_not_retried_and_invalidates_resolution() {
    let selected = selector(1);
    let mut core = core([vec![observed(selected, 1, "Device")]]);
    core.io.writes.push_back(Err("driver failed".into()));
    core.refresh();

    assert!(matches!(
        core.send_report(&device(selected), &[1]),
        Err(HidError::Write { .. })
    ));
    assert_eq!(
        core.io
            .operations
            .iter()
            .filter(|operation| matches!(operation, Operation::Write(..)))
            .count(),
        1
    );
    assert!(!core.locators.contains_key(&selected));
    assert!(matches!(
        core.send_report(&device(selected), &[1]),
        Err(HidError::ResolutionInvalidated { .. })
    ));
    assert_eq!(
        core.io
            .operations
            .iter()
            .filter(|operation| matches!(operation, Operation::Enumerate))
            .count(),
        1
    );
}

#[test]
fn short_write_is_not_retried_and_invalidates_resolution() {
    let selected = selector(1);
    let mut core = core([vec![observed(selected, 1, "Device")]]);
    core.io.writes.push_back(Ok(2));
    core.refresh();

    let error = core.send_report(&device(selected), &[1]).unwrap_err();

    assert_eq!(
        error,
        HidError::ShortWrite {
            selector: selected,
            expected: 5,
            actual: 2,
        }
    );
    assert_eq!(
        core.io
            .operations
            .iter()
            .filter(|operation| matches!(operation, Operation::Write(..)))
            .count(),
        1
    );
    assert!(!core.locators.contains_key(&selected));
}

#[test]
fn failed_refresh_retains_stale_rows_but_clears_locators_and_presence() {
    let selected = selector(1);
    let mut io = FakeIo::new([vec![observed(selected, 1, "Device")]]);
    io.enumerations.push_back(Err(HidError::Enumeration {
        message: "unavailable".into(),
    }));
    let mut core = HidCore::new(io, FakeClock::new());
    let ready = core.refresh();

    let failed = core.refresh();

    assert_eq!(ready.revision, 1);
    assert_eq!(failed.revision, 2);
    assert_eq!(failed.rows, ready.rows);
    assert!(matches!(
        failed.refresh_state,
        HidRefreshState::Failed { .. }
    ));
    assert_eq!(failed.presence(&device(selected)), HidPresence::Unknown);
    assert!(core.locators.is_empty());
}

#[test]
fn ready_inventory_distinguishes_connected_disconnected_and_ambiguous_presence() {
    let connected = selector(1);
    let ambiguous = selector(10);
    let mut core = core([vec![
        observed(connected, 1, "Connected"),
        observed(ambiguous, 2, "Duplicate"),
        observed(ambiguous, 3, "Duplicate"),
    ]]);
    let inventory = core.refresh();

    assert_eq!(
        inventory.presence(&device(connected)),
        HidPresence::Connected
    );
    assert_eq!(
        inventory.presence(&device(selector(20))),
        HidPresence::Disconnected
    );
    assert_eq!(
        inventory.presence(&device(ambiguous)),
        HidPresence::Ambiguous { matches: 2 }
    );
}
