//! 只含元数据的持久审计结果与权威状态变更一同提交。
use crate::{ControlDb, DbError};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 待投递的运行时事实（非接口端点审计），与其数据库状态迁移原子写入。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAuditRecord {
    /// 供账本重放的稳定标识。
    pub event_id: String,
    /// 稳定的动作名。
    pub action: String,
    /// 只含已净化的元数据，绝不含产物或笔记正文。
    pub payload: Value,
}
pub(super) fn enqueue(
    tx: &Transaction<'_>,
    event_id: &str,
    action: &str,
    payload: Value,
) -> Result<(), DbError> {
    tx.execute(
        "INSERT OR IGNORE INTO runtime_audit_outbox(event_id,action,payload_json) VALUES(?1,?2,?3)",
        params![event_id, action, serde_json::to_string(&payload)?],
    )?;
    Ok(())
}
impl ControlDb {
    /// 列出审计接收端尚未确认的持久运行时事实。
    pub fn pending_runtime_audits(&self) -> Result<Vec<RuntimeAuditRecord>, DbError> {
        let mut statement=self.connection.prepare("SELECT event_id,action,payload_json FROM runtime_audit_outbox WHERE delivered=0 ORDER BY sequence")?;
        let rows = statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (event_id, action, payload) = row?;
            events.push(RuntimeAuditRecord {
                event_id,
                action,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(events)
    }
    /// 只在权威审计追加成功后调用。
    pub fn acknowledge_runtime_audit(&mut self, event_id: &str) -> Result<(), DbError> {
        self.connection.execute(
            "UPDATE runtime_audit_outbox SET delivered=1 WHERE event_id=?1",
            [event_id],
        )?;
        Ok(())
    }
}
