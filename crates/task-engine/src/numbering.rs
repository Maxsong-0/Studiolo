//! 命名空间编号分配始终以 Broker 中真实的文件名为准，从不采信客户端给出的最大值。
use super::*;
use studiolo_vault_broker::NumberingScanner;
impl<B: VaultBroker + ArtifactStager + NumberingScanner, L: AuditLedger>
    PersistentTaskEngine<B, L>
{
    /// 先扫描规范文件名，再原子预留下一个编号，重放结果稳定。
    pub fn reserve_number(
        &mut self,
        request_id: &str,
        task: TaskId,
        scope: &VaultPath,
        prefix: &str,
    ) -> Result<u64, TaskEngineError> {
        if request_id.is_empty()
            || request_id.len() > 256
            || prefix.is_empty()
            || prefix.len() > 64
            || prefix.contains('/')
        {
            return Err(TaskEngineError::InvalidTaskSpec(
                "invalid numbering request",
            ));
        }
        if self.control_db.board_task(task).map_err(map_db)?.is_none() {
            return Err(TaskEngineError::NotFound("board task"));
        }
        let request = (request_id, task, scope, prefix);
        self.audit_board("task.number_reserved", &request, AuditPhase::Intent, None)?;
        let maximum = self
            .broker
            .scan_number_max(scope, prefix)
            .map_err(TaskEngineError::Broker)?;
        let result = self
            .control_db
            .reserve_board_number(
                request_id,
                task,
                &format!("{}:{}", scope.collision_key(), prefix.to_lowercase()),
                maximum,
            )
            .map_err(map_db);
        self.audit_board(
            "task.number_reserved",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
}
