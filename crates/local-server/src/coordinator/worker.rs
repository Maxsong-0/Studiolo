//! 捕获、纯派生构建与最终发布；运行位只由实际作业归还，作业投递不出去时由收尾线程代还。

use std::sync::Arc;

use studiolo_vault_broker::{BrokerError, ScanLimits};

use super::{Core, Failure, WatchHealth};
use crate::{
    deriver::OnDeriveThread,
    view::{self, Freshness},
    watcher,
};

/// 作业投递不出去时代做收尾的短命线程（Linux 线程名上限 15 字节）。
const CLOSING_THREAD_NAME: &str = "studiolo-close";

impl Core {
    pub(super) fn run(self: &Arc<Self>, on: &OnDeriveThread) {
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
            let result = self.attempt(on);
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

    fn attempt(self: &Arc<Self>, on: &OnDeriveThread) -> Result<(), Failure> {
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
            self.enable_approved_file_operations(broker)?;
            if self.content_audit.is_some() {
                // 普通正文审计降级不阻止读取派生；恢复未闭合的写入门禁由持久状态决定。
                let _ = self.flush_content_audit(broker);
                let _ = self.flush_file_operation_audit(broker);
            }
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
            on,
        )
        .map_err(|_| Failure::Build)?;
        #[cfg(test)]
        self.probe.built();
        let owner = self.owner.lock().map_err(|_| Failure::Internal)?;
        let broker = owner.as_ref().ok_or(Failure::Stopped)?;
        self.roots.revalidate().map_err(|_| Failure::Binding)?;
        broker.revalidate_capture(&stamp).map_err(classify)?;
        let writer_status = broker.write_status().map_err(classify)?;
        if writer_status.read_blocked {
            return Err(Failure::Unresolved);
        }
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
            stale: writer_status.unresolved,
            indexed_through: Some(stamp.committed_through().to_string()),
            committed_through: Some(stamp.committed_through().to_string()),
            unresolved: Some(writer_status.unresolved),
            watcher_healthy: true,
            reason: writer_status.unresolved.then_some("unresolved"),
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
        self.release_run();
    }

    /// 交还运行位：已受理的请求都有了结论，等待者与关停据此返回。
    fn release_run(&self) {
        let mut state = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.running = false;
        state.pending = false;
        state.finished = state.requested;
        self.announce(&state);
    }

    /// 专用派生线程不在了、作业投递不出去：与作业 panic 一样失败关闭——不再受理触发、
    /// 等待者立刻得到失败、视图标 stale；watcher 与 Broker 照样收尾后才交还运行位，
    /// 所以关停返回时租约已经释放。请求刷新的一方可能正持 owner 锁（正文保存、恢复、
    /// 文件操作与 Bridge 写入都在锁内请求刷新），也可能身处 notify 回调线程，
    /// 所以收尾另起一条短命线程做，等它放下 owner 锁。
    pub(super) fn abandon_launch(self: &Arc<Self>) {
        {
            let mut state = self
                .control
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closing = true;
            state.result = Some(Failure::Internal);
            state.health = WatchHealth::Stopped;
            self.mark_stale(&state, "unavailable");
            self.announce(&state);
        }
        let core = Arc::clone(self);
        let closing = std::thread::Builder::new()
            .name(CLOSING_THREAD_NAME.to_owned())
            .spawn(move || core.cleanup());
        if closing.is_err() {
            // 收尾线程也起不来：只交还运行位，等待者与关停不至于挂住；watcher 与租约随 Core 析构释放。
            self.release_run();
        }
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
