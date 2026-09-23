//! 封闭的 canonical 拥有者；公开控制句柄不导出 Broker 或任意写回调。

mod content;
mod content_audit;
mod file_operations;
mod history;
mod observation;
mod preparation;
mod scheduler;
mod sync_bridge;
mod worker;

use std::sync::{Arc, Mutex};

use studiolo_core::VaultPath;
use studiolo_host_config::CanonicalServingRoots;
use studiolo_vault_broker::{CanonicalRead, FsVaultBroker};
use tokio::sync::watch;

use crate::{LocalError, view::ViewStore};

/// 显式写者模式的工作台生命周期；通过 Arc 共享句柄，始终只有一个 Broker 实例。
pub struct CanonicalWorkbench {
    pub(crate) core: Arc<Core>,
}

pub(crate) struct Core {
    pub owner: Mutex<Option<FsVaultBroker>>,
    pub roots: Arc<CanonicalServingRoots>,
    pub view: ViewStore,
    pub control: Mutex<Control>,
    pub watcher: Mutex<Option<notify::RecommendedWatcher>>,
    pub completion: watch::Sender<Completion>,
    pub executor: tokio::runtime::Handle,
    pub content_audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>>,
    pub content_audit_healthy: std::sync::atomic::AtomicBool,
    pub content_restore_pending: std::sync::atomic::AtomicBool,
    pub file_operation_pending: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub probe: crate::coordinator_tests::Probe,
}

pub(crate) struct Control {
    pub closing: bool,
    pub running: bool,
    pub pending: bool,
    pub requested: u64,
    pub finished: u64,
    pub epoch: u64,
    pub watch_generation: u64,
    pub health: WatchHealth,
    pub result: Option<Failure>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchHealth {
    Starting,
    Healthy,
    Failed,
    Stopped,
}

#[derive(Clone, Copy)]
pub(crate) struct Completion {
    finished: u64,
    running: bool,
    closed: bool,
    result: Option<Failure>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Failure {
    Changed,
    Watcher,
    Capture,
    Build,
    Binding,
    Unresolved,
    Writer,
    Stopped,
    Internal,
}

impl Failure {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Changed => "changed",
            Self::Watcher => "watcher_failed",
            Self::Capture => "capture_failed",
            Self::Build => "build_failed",
            Self::Binding => "binding_changed",
            Self::Unresolved => "unresolved",
            Self::Writer => "writer_state_invalid",
            Self::Stopped => "stopping",
            Self::Internal => "unavailable",
        }
    }
}

impl CanonicalWorkbench {
    /// 监听前检查批准的根、现持 Broker 身份与已知未决状态；不签发编辑权限。
    pub fn revalidate(&self) -> Result<(), LocalError> {
        if self
            .core
            .control
            .lock()
            .map_err(|_| LocalError::Unavailable)?
            .closing
        {
            return Err(LocalError::Unavailable);
        }
        let owner = self
            .core
            .owner
            .lock()
            .map_err(|_| LocalError::Unavailable)?;
        let broker = owner.as_ref().ok_or(LocalError::Unavailable)?;
        let result = (|| {
            self.core
                .roots
                .revalidate()
                .map_err(|_| LocalError::Configuration)?;
            broker.validate_root_identities(
                self.core.roots.vault_identity(),
                self.core.roots.writer_data_identity(),
            )?;
            let status = broker.write_status()?;
            if status.unresolved {
                return Err(LocalError::Read(
                    studiolo_vault_broker::BrokerError::MutationUnresolved,
                ));
            }
            let state = self
                .core
                .control
                .lock()
                .map_err(|_| LocalError::Unavailable)?;
            let view = self
                .core
                .view
                .0
                .read()
                .map_err(|_| LocalError::Unavailable)?;
            if state.closing
                || state.health != WatchHealth::Healthy
                || view
                    .freshness
                    .as_ref()
                    .is_none_or(|freshness| freshness.stale)
                || view
                    .generation
                    .as_ref()
                    .is_none_or(|generation| generation.invalidation_epoch != Some(state.epoch))
            {
                return Err(LocalError::Unavailable);
            }
            if view.generation.as_ref().is_none_or(|generation| {
                generation.indexed_through != Some(status.committed_through)
            }) {
                return Err(LocalError::Read(
                    studiolo_vault_broker::BrokerError::CaptureChanged,
                ));
            }
            Ok(())
        })();
        if let Err(error) = &result {
            self.core.observe_error(error);
        }
        result
    }

    /// 幂等停止新触发并唤醒请求等待者；native watcher 的关闭由实际后台作业处理。
    pub fn stop(&self) {
        self.core.stop();
    }

    /// 停止周期触发和监视，等待真正的捕获/构建及清理作业结束后释放 Broker 租约。
    pub async fn shutdown(&self) -> Result<(), LocalError> {
        self.stop();
        let mut changed = self.core.completion.subscribe();
        loop {
            let state = *changed.borrow_and_update();
            if state.closed && !state.running {
                return Ok(());
            }
            changed
                .changed()
                .await
                .map_err(|_| LocalError::Unavailable)?;
        }
    }

    pub(crate) async fn refresh(&self) -> Result<(), LocalError> {
        let ticket = self.core.request("refreshing")?;
        self.core.wait(ticket).await
    }

    pub(crate) fn read_file(
        &self,
        path: &VaultPath,
        max_bytes: u64,
    ) -> Result<CanonicalRead, LocalError> {
        let owner = self
            .core
            .owner
            .lock()
            .map_err(|_| LocalError::Unavailable)?;
        let broker = owner.as_ref().ok_or(LocalError::Unavailable)?;
        if self
            .core
            .control
            .lock()
            .map_err(|_| LocalError::Unavailable)?
            .closing
        {
            return Err(LocalError::Unavailable);
        }
        let result = (|| {
            self.core
                .roots
                .revalidate()
                .map_err(|_| LocalError::Configuration)?;
            let note = broker.read_file(path, max_bytes)?;
            self.core
                .roots
                .revalidate()
                .map_err(|_| LocalError::Configuration)?;
            Ok(note)
        })();
        if let Err(error) = &result {
            self.core.observe_error(error);
        }
        result
    }
}

impl Drop for CanonicalWorkbench {
    fn drop(&mut self) {
        self.stop();
    }
}
