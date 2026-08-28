use std::{sync::Arc, thread};

use anyhow::{Context, Result};
use tokio::sync::watch;

use crate::{automation_runtime::AutomationRuntime, config::PublishedConfig};

pub(crate) struct ConfigRuntimeBridge {
    shutdown: watch::Sender<bool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ConfigRuntimeBridge {
    pub(crate) fn start(
        runtime: AutomationRuntime,
        mut publications: watch::Receiver<Arc<PublishedConfig>>,
    ) -> Result<Self> {
        // Catch up synchronously so a publication between runtime startup and thread startup
        // cannot leave the runtime on an obsolete snapshot.
        runtime.replace_active_config(publications.borrow_and_update().active().clone());
        let (shutdown, mut shutdown_rx) = watch::channel(false);
        let executor = tokio::runtime::Builder::new_current_thread()
            .build()
            .context("Failed to create configuration runtime bridge executor")?;
        let worker = thread::Builder::new()
            .name("locked-in-config-runtime".to_string())
            .spawn(move || {
                executor.block_on(async move {
                    loop {
                        tokio::select! {
                            biased;
                            changed = shutdown_rx.changed() => {
                                if changed.is_err() || *shutdown_rx.borrow_and_update() {
                                    break;
                                }
                            }
                            changed = publications.changed() => {
                                if changed.is_err() {
                                    break;
                                }
                                let publication = publications.borrow_and_update().clone();
                                runtime.replace_active_config(publication.active().clone());
                            }
                        }
                    }
                });
            })
            .context("Failed to start configuration runtime bridge")?;
        Ok(Self {
            shutdown,
            worker: Some(worker),
        })
    }

    pub(crate) fn shutdown_and_join(mut self) {
        self.shutdown.send_replace(true);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for ConfigRuntimeBridge {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

#[cfg(test)]
mod tests;
