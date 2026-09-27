use std::{future::Future, sync::Mutex, time::Duration};

use crate::{
    app_log, automation_runtime::RuntimeOwner, config_runtime_bridge::ConfigRuntimeBridge,
};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Owns backend lifetimes until desktop exit has finished waiting for them.
pub(crate) struct ApplicationLifecycle {
    workers: Mutex<Option<Workers>>,
}

struct Workers {
    runtime: RuntimeOwner,
    config_bridge: Option<ConfigRuntimeBridge>,
}

impl ApplicationLifecycle {
    pub(crate) fn new(owner: RuntimeOwner, config_bridge: Option<ConfigRuntimeBridge>) -> Self {
        Self {
            workers: Mutex::new(Some(Workers {
                runtime: owner,
                config_bridge,
            })),
        }
    }

    /// Closes admission immediately. Only the first request owns completion;
    /// repeated close/quit events must not terminate the desktop ahead of it.
    pub(crate) fn begin_shutdown(&self) -> Option<impl Future<Output = ()> + 'static> {
        self.begin_shutdown_with_timeout(SHUTDOWN_TIMEOUT)
    }

    fn begin_shutdown_with_timeout(
        &self,
        timeout: Duration,
    ) -> Option<impl Future<Output = ()> + 'static> {
        let workers = self
            .workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()?;
        workers.runtime.request_shutdown();
        app_log::write("application shutdown requested");
        let deadline = tokio::time::Instant::now() + timeout;
        Some(async move {
            // The UI executor stays responsive while synchronous joins wait.
            let joins = tokio::task::spawn_blocking(move || {
                if let Some(bridge) = workers.config_bridge {
                    bridge.shutdown_and_join();
                }
                workers.runtime.shutdown_and_join(
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                );
            });
            // The deadline includes both joins and blocking-pool scheduling.
            // Timing out detaches the join task; it does not cancel driver I/O.
            match tokio::time::timeout_at(deadline, joins).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    app_log::write_error(format!("application shutdown failed: {error}"))
                }
                Err(_) => app_log::write_error("application backend shutdown timed out"),
            }
            app_log::write("application shutdown wait finished");
        })
    }
}

#[cfg(test)]
mod tests;
