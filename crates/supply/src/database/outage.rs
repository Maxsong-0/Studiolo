//! 审计故障窗口索引，保留原有事务和有界计数语义。
use super::{AgentStore, SupplyError, unavailable};
use rusqlite::params;

impl AgentStore {
    /// 记录一次审计不可用窗口的开始（有界补记：同一 outage 只记一行）。
    pub fn record_outage_started(
        &self,
        outage_id: &str,
        now: &str,
        reason: &str,
    ) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT OR IGNORE INTO outage (id, started_at, ended_at, denied_count, reason)
                 VALUES (?1, ?2, NULL, 0, ?3)",
                params![outage_id, now, reason],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 降级窗口内每次拒绝加一（有界：计数封顶于 i64）。
    pub fn record_outage_denial(&self, outage_id: &str) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE outage SET denied_count = MIN(denied_count + 1, 9223372036854775807) WHERE id = ?1",
                params![outage_id],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 标记 outage 结束。
    pub fn record_outage_ended(&self, outage_id: &str, now: &str) -> Result<(), SupplyError> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE outage SET ended_at = ?2 WHERE id = ?1 AND ended_at IS NULL",
                params![outage_id, now],
            )
            .map_err(unavailable)?;
        Ok(())
    }

    /// 全部 outage 记录：(id, started_at, ended_at, denied_count, reason)。
    #[allow(clippy::type_complexity)]
    pub fn outages(
        &self,
    ) -> Result<Vec<(String, String, Option<String>, i64, String)>, SupplyError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, started_at, ended_at, denied_count, reason FROM outage ORDER BY started_at")
            .map_err(unavailable)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(unavailable)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
    }
}
