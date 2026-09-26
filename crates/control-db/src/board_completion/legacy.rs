//! 旧版内联终态正文的一次性迁移：列出待迁记录，Broker staging 之后换成指针。
use super::is_pointer_reservation;
use crate::{
    AttemptRecord, ControlDb, DbError, TerminalReservation,
    types::{parse_attempt_id, parse_task_id},
};
use rusqlite::params;
use studiolo_core::AttemptId;

impl ControlDb {
    /// 列出旧版内联终态正文，供经 Broker staging 的一次性迁移使用。
    pub fn legacy_terminal_attempts(&self) -> Result<Vec<AttemptRecord>, DbError> {
        let mut stmt = self.connection.prepare("SELECT task_id,attempt_id FROM attempts WHERE terminal_json IS NOT NULL AND json_type(terminal_json,'$.artifacts_json')='array'")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(task, attempt)| {
                self.attempt(parse_task_id(&task)?, parse_attempt_id(&attempt)?)?
                    .ok_or(DbError::NotFound("legacy attempt"))
            })
            .collect()
    }

    /// 只在 Broker 已持久 staging 之后，替换未变化的历史正文。
    pub fn migrate_terminal_pointer(
        &mut self,
        attempt_id: AttemptId,
        expected: &TerminalReservation,
        pointer: &TerminalReservation,
    ) -> Result<(), DbError> {
        if !is_pointer_reservation(pointer) {
            return Err(DbError::BoardRejected("migration pointer has inline data"));
        }
        let changed = self.connection.execute(
            "UPDATE attempts SET terminal_json=?3 WHERE attempt_id=?1 AND terminal_json=?2",
            params![
                attempt_id.to_string(),
                serde_json::to_string(expected)?,
                serde_json::to_string(pointer)?
            ],
        )?;
        if changed != 1 {
            return Err(DbError::IdempotencyMismatch);
        }
        Ok(())
    }
}
