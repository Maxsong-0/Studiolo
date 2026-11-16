//! 只有可信启动协调器能固定产物批准；任何正文提交都须读回原记录。
use super::*;
use studiolo_core::ArtifactProvenance;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 在启动执行体之前保存原批准并确认审计；失败不得继续发放能力或启动进程。
    /// 重放只确认原批准，不代表 lease 或正文留存期仍有效。
    #[allow(clippy::too_many_arguments)]
    pub fn authorize_artifacts(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        snapshot: &str,
        provenance: &ArtifactProvenance,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        self.verify_vault_binding()?;
        self.control_db
            .authorize_artifacts(task, attempt, fence, snapshot, provenance, now_ms)
            .map_err(map_db)?;
        self.deliver_runtime_audits()?;
        Ok(())
    }
}
