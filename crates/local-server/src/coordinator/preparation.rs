//! 单一writer的可信启动与首次完整捕获；审计能力只由装配层注入。

use super::*;

impl CanonicalWorkbench {
    /// 消费已打开的唯一 Broker；先注册监视，再完成首个完整视图，失败不提供路由能力。
    pub async fn prepare(
        broker: FsVaultBroker,
        roots: Arc<CanonicalServingRoots>,
    ) -> Result<Arc<Self>, LocalError> {
        Self::prepare_inner(broker, roots, None).await
    }

    /// 可信装配增加审计能力；写权限仍只来自私有配置的绑定范围。
    pub async fn prepare_with_audit(
        broker: FsVaultBroker,
        roots: Arc<CanonicalServingRoots>,
        audit: Arc<dyn studiolo_audit_ledger::AuditLedger>,
    ) -> Result<Arc<Self>, LocalError> {
        Self::prepare_inner(broker, roots, Some(audit)).await
    }

    async fn prepare_inner(
        broker: FsVaultBroker,
        roots: Arc<CanonicalServingRoots>,
        audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>>,
    ) -> Result<Arc<Self>, LocalError> {
        let (completion, _) = watch::channel(Completion {
            finished: 0,
            running: false,
            closed: false,
            result: None,
        });
        let workbench = Arc::new(Self {
            core: Arc::new(Core {
                owner: Mutex::new(Some(broker)),
                roots,
                view: ViewStore::canonical(),
                control: Mutex::new(Control {
                    closing: false,
                    running: false,
                    pending: false,
                    requested: 0,
                    finished: 0,
                    epoch: 0,
                    watch_generation: 0,
                    health: WatchHealth::Starting,
                    result: None,
                }),
                watcher: Mutex::new(None),
                completion,
                executor: tokio::runtime::Handle::current(),
                content_audit: audit,
                content_audit_healthy: std::sync::atomic::AtomicBool::new(true),
                content_restore_pending: std::sync::atomic::AtomicBool::new(false),
                file_operation_pending: std::sync::atomic::AtomicBool::new(false),
                #[cfg(test)]
                probe: crate::coordinator_tests::Probe::default(),
            }),
        });
        let ticket = workbench.core.request("initializing")?;
        if workbench.core.wait(ticket).await.is_err() || workbench.revalidate().is_err() {
            workbench.shutdown().await?;
            return Err(LocalError::Unavailable);
        }
        scheduler::periodic(&workbench.core);
        Ok(workbench)
    }
}
