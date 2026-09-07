//! 捕获、纯派生构建与最终发布；只有此实际作业归还运行位。

use std::sync::Arc;

use studiolo_vault_broker::{BrokerError, ScanLimits};

use super::{Core, Failure, WatchHealth};
use crate::{
    view::{self, Freshness},
    watcher,
};

impl Core {
    pub(super) fn run(self: &Arc<Self>) {
        loop {
            let ticket = {
                let mut state = self
                    .control
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.closing {
                    drop(state);
                    self.cleanup();
                    return;
                }
                if !state.pending {
                    state.running = false;
                    self.announce(&state);
                    return;
                }
                state.pending = false;
                state.requested
            };
            let result = self.attempt();
            let mut state = self
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.finished = state.finished.max(ticket);
            state.result = result.err();
            if let Some(failure) = state.result {
                self.mark_stale(&state, failure.reason());
                // 仅确认本 ticket；执行期间的新请求保留一个 pending，没有新请求就停止。
            }
            self.announce(&state);
        }
    }

    fn attempt(self: &Arc<Self>) -> Result<(), Failure> {
        watcher::ensure(self)?;
        let (capture, epoch, next) = {
            let mut owner = self.owner.lock().map_err(|_| Failure::Internal)?;
            let broker = owner.as_mut().ok_or(Failure::Stopped)?;
            self.roots.revalidate().map_err(|_| Failure::Binding)?;
            broker
                .validate_root_identities(
                    self.roots.vault_identity(),
                    self.roots.writer_data_identity(),
                )
                .map_err(classify)?;
            let state = self.control.lock().map_err(|_| Failure::Internal)?;
            if state.closing {
                return Err(Failure::Stopped);
            }
            let epoch = state.epoch;
            let next = self
                .view
                .0
                .read()
                .map_err(|_| Failure::Internal)?
                .generation
                .as_ref()
                .map_or(0, |view| view.number)
                .checked_add(1)
                .ok_or(Failure::Internal)?;
            drop(state);
            #[cfg(test)]
            self.probe.capture();
            (
                broker.capture(ScanLimits::default()).map_err(classify)?,
                epoch,
                next,
            )
        };
        let stamp = capture.stamp;
        #[cfg(test)]
        self.probe.building();
        let mut candidate = view::build(
            capture.entries,
            capture
                .notes
                .into_iter()
                .map(|note| (note.path, note.content)),
            next,
        )
        .map_err(|_| Failure::Build)?;
        #[cfg(test)]
        self.probe.built();
        let owner = self.owner.lock().map_err(|_| Failure::Internal)?;
        let broker = owner.as_ref().ok_or(Failure::Stopped)?;
        self.roots.revalidate().map_err(|_| Failure::Binding)?;
        broker.revalidate_capture(&stamp).map_err(classify)?;
        let state = self.control.lock().map_err(|_| Failure::Internal)?;
        if state.closing {
            return Err(Failure::Stopped);
        }
        if state.epoch != epoch || state.health != WatchHealth::Healthy {
            return Err(Failure::Changed);
        }
        candidate.indexed_through = Some(stamp.committed_through());
        candidate.invalidation_epoch = Some(epoch);
        let mut published = self.view.0.write().map_err(|_| Failure::Internal)?;
        let previous = published.generation.replace(Arc::new(candidate));
        published.freshness = Some(Freshness {
            stale: false,
            indexed_through: Some(stamp.committed_through().to_string()),
            committed_through: Some(stamp.committed_through().to_string()),
            unresolved: Some(false),
            watcher_healthy: true,
            reason: None,
        });
        drop(published);
        drop(state);
        drop(owner);
        drop(previous);
        Ok(())
    }

    fn cleanup(&self) {
        // drop native watcher 可能等待回调线程，绝不能持 control/view 或在事件循环中执行。
        let watcher = self
            .watcher
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(watcher);
        let owner = self
            .owner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(owner);
        let mut state = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.running = false;
        state.pending = false;
        state.finished = state.requested;
        self.announce(&state);
    }

    pub(super) fn panic_cleanup(&self) {
        {
            let mut state = self
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closing = true;
            state.result = Some(Failure::Internal);
            state.health = WatchHealth::Stopped;
            self.mark_stale(&state, "unavailable");
        }
        self.cleanup();
    }
}

fn classify(error: BrokerError) -> Failure {
    match error {
        BrokerError::CaptureChanged | BrokerError::UnstableRead(_) => Failure::Changed,
        BrokerError::RootChanged
        | BrokerError::WriterBindingMismatch
        | BrokerError::CorruptIdentity => Failure::Binding,
        BrokerError::MutationUnresolved => Failure::Unresolved,
        BrokerError::CorruptWriterState => Failure::Writer,
        _ => Failure::Capture,
    }
}
