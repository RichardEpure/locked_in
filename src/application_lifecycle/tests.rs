use std::sync::mpsc;

use super::*;
use crate::{
    automation_runtime::{
        AutomationRuntime, EventSourceState, FocusInput, RuntimeInputs, RuntimePhase,
        RuntimeRequestError,
    },
    config::Device,
    focused_window::ForegroundObservation,
    hid::{HidBackend, HidError, HidInventory, HidRefreshState},
};

struct BlockedBackend {
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl HidBackend for BlockedBackend {
    fn inventory(&self) -> HidInventory {
        HidInventory::default()
    }

    fn refresh(&mut self) -> HidInventory {
        self.started.send(()).unwrap();
        self.release.recv().unwrap();
        HidInventory {
            refresh_state: HidRefreshState::Ready,
            ..HidInventory::default()
        }
    }

    fn send_report(&mut self, _: &Device, _: &[u8]) -> Result<(), HidError> {
        unreachable!("shutdown must prevent dispatch")
    }
}

fn blocked_application() -> (ApplicationLifecycle, AutomationRuntime, mpsc::Sender<()>) {
    let (started, started_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let (_, observations) = tokio::sync::watch::channel(ForegroundObservation::default());
    let (runtime, owner) = AutomationRuntime::start_active(
        None,
        RuntimeInputs {
            focused_window: FocusInput::new(observations, EventSourceState::Available),
        },
        BlockedBackend {
            started,
            release: release_rx,
        },
    )
    .unwrap();
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    (ApplicationLifecycle::new(owner, None), runtime, release)
}

#[test]
fn exit_closes_admission_once_and_yields_until_the_in_flight_batch_finishes() {
    let (lifecycle, runtime, release) = blocked_application();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    executor.block_on(async {
        let completion = lifecycle.begin_shutdown().unwrap();
        assert!(lifecycle.begin_shutdown().is_none());
        assert_eq!(
            runtime.subscribe_status().borrow().phase,
            RuntimePhase::Stopping
        );
        assert_eq!(
            runtime.request_hid_refresh(),
            Err(RuntimeRequestError::Unavailable)
        );

        let mut completion = Box::pin(completion);
        // A timer on this same executor can progress while the HID thread is
        // blocked; desktop completion must still be pending.
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut completion)
                .await
                .is_err()
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), completion)
            .await
            .unwrap();
        assert_eq!(
            runtime.subscribe_status().borrow().phase,
            RuntimePhase::Stopped
        );
    });
}

#[test]
fn exit_finishes_waiting_after_timeout_without_claiming_a_blocked_worker_stopped() {
    let (lifecycle, runtime, release) = blocked_application();
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    executor.block_on(async {
        let completion = lifecycle
            .begin_shutdown_with_timeout(Duration::from_millis(20))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), completion)
            .await
            .unwrap();
        assert_eq!(
            runtime.subscribe_status().borrow().phase,
            RuntimePhase::Stopping
        );
        assert!(lifecycle.begin_shutdown().is_none());

        let mut status = runtime.subscribe_status();
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while status.borrow_and_update().phase != RuntimePhase::Stopped {
                status.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    });
}

#[test]
fn overall_deadline_includes_a_blocked_configuration_bridge() {
    let (mut lifecycle, runtime, release_hid) = blocked_application();
    let (bridge, release_bridge) = ConfigRuntimeBridge::blocked_for_test();
    lifecycle
        .workers
        .get_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .config_bridge = Some(bridge);
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    executor.block_on(async {
        let completion = lifecycle
            .begin_shutdown_with_timeout(Duration::from_millis(20))
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), completion).await;
        // Release both gates even if the deadline assertion fails, so executor
        // teardown cannot hang on its blocking pool.
        release_bridge.send(()).unwrap();
        release_hid.send(()).unwrap();
        assert!(
            result.is_ok(),
            "configuration join exceeded the application deadline"
        );
        assert!(lifecycle.begin_shutdown().is_none());
        assert_eq!(
            runtime.request_hid_refresh(),
            Err(RuntimeRequestError::Unavailable)
        );
    });
}
