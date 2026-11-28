//! 命名空间编号分配始终以 Broker 中真实的文件名为准，从不采信客户端给出的最大值。
use super::*;
use crate::board_audit::BoardVerdict;
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
        let audit = self.board_intent("task.number_reserved", &request)?;
        let namespace = format!("{}:{}", scope.collision_key(), prefix.to_lowercase());
        // 重放先查登记：发过的号原样返回、不再扫库，库里的范围目录后来没了也不影响。
        let result = match self
            .control_db
            .board_number_replay(request_id, task, &namespace)
            .map_err(map_db)
        {
            Ok(Some(number)) => Ok(number),
            Err(error) => Err(error),
            // 扫描失败（范围目录不存在、扫描不完整）和控制库忙一样没下结论、不记 Outcome：
            // 条件满足后同一请求还能成功（BUG-038、BUG-060）。
            Ok(None) => match self.broker.scan_number_max(scope, prefix) {
                Ok(maximum) => self
                    .control_db
                    .reserve_board_number(request_id, task, &namespace, maximum)
                    .map_err(map_db),
                Err(error) => Err(TaskEngineError::Broker(error)),
            },
        };
        self.board_outcome(audit, BoardVerdict::of(&result))?;
        result
    }
}
