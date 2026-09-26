//! 终态 T1：预留终态赢家及其前滚输入。
use crate::{
    ControlDb, DbError, TerminalReservation, board, board_completion, lifecycle,
    projection_outbox::enqueue_projection, stopped_candidates, types::fence_i64,
};
use rusqlite::{OptionalExtension as _, TransactionBehavior, params};
use serde_json::Value;
use studiolo_core::{AttemptId, Fence, TaskId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TerminalKind {
    Legacy,
    BoardReview,
    BoardStopped,
}
impl TerminalKind {
    fn outcome(self) -> Option<Value> {
        (self == Self::BoardStopped).then(|| Value::String("stopped".into()))
    }
    fn required_phase(self) -> &'static str {
        if self == Self::BoardStopped {
            "cancel_requested"
        } else {
            "running"
        }
    }
}

impl ControlDb {
    /// 执行 T1：预留终态赢家及其前滚输入。
    pub fn reserve_terminal(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.reserve_terminal_kind(
            task_id,
            attempt_id,
            fence,
            reservation,
            now_ms,
            TerminalKind::Legacy,
        )
    }

    /// 沿用既有 T1 协议，为需主人审阅的 runner 候选预留终态。
    pub fn reserve_board_runner_candidates(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now_ms: i64,
    ) -> Result<(), DbError> {
        self.reserve_terminal_kind(
            task_id,
            attempt_id,
            fence,
            reservation,
            now_ms,
            TerminalKind::BoardReview,
        )
    }

    pub(super) fn reserve_terminal_kind(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        reservation: &TerminalReservation,
        now_ms: i64,
        kind: TerminalKind,
    ) -> Result<(), DbError> {
        if !board_completion::is_pointer_reservation(reservation)
            || reservation.report_json.get("outcome") != kind.outcome().as_ref()
        {
            return Err(DbError::BoardRejected(
                "new terminal reservations must contain metadata and staging pointers only",
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let state: Option<(String, i64, Option<String>)> = tx
            .query_row(
                "SELECT phase, fence, terminal_json FROM attempts
                 WHERE task_id = ?1 AND attempt_id = ?2",
                params![task_id.to_string(), attempt_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let encoded = serde_json::to_string(reservation)?;
        match state {
            Some((_, stored_fence, _)) if stored_fence != fence_i64(fence)? => {
                return Err(DbError::FenceMismatch);
            }
            Some((phase, _, prior))
                if phase == "committing" && prior.as_deref() == Some(encoded.as_str()) =>
            {
                tx.commit()?;
                return Ok(());
            }
            Some((phase, _, _)) if phase != kind.required_phase() => {
                return Err(DbError::InvalidTransition {
                    from: phase,
                    operation: "reserve_terminal",
                });
            }
            None => return Err(DbError::NotFound("attempt")),
            _ => {}
        }
        if kind == TerminalKind::BoardStopped {
            stopped_candidates::require_cancel_window(&tx, task_id, attempt_id, fence, now_ms)?;
        } else {
            lifecycle::require_live(&tx, task_id, attempt_id, fence, now_ms)?;
        }
        board::require_children_terminal(&tx, task_id)?;
        let is_board = tx.query_row(
            "SELECT COUNT(*) FROM board_tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get::<_, i64>(0),
        )? != 0;
        if is_board != (kind != TerminalKind::Legacy) {
            return Err(DbError::BoardRejected(
                "board candidates require the runner review protocol",
            ));
        }
        if kind != TerminalKind::Legacy {
            let adapter: String = tx.query_row(
                "SELECT json_extract(definition_json,'$.adapter') FROM tasks WHERE task_id=?1",
                [task_id.to_string()],
                |r| r.get(0),
            )?;
            if adapter == "consumer" {
                return Err(DbError::BoardRejected(
                    "consumer task cannot stage runner candidates",
                ));
            }
        }
        tx.execute(
            "UPDATE attempts SET phase = 'committing', terminal_json = ?4,
                 updated_at_ms = ?5 WHERE task_id = ?1 AND attempt_id = ?2 AND fence = ?3",
            params![
                task_id.to_string(),
                attempt_id.to_string(),
                fence_i64(fence)?,
                encoded,
                now_ms
            ],
        )?;
        tx.execute(
            "UPDATE tasks SET status = 'committing', updated_at_ms = ?2 WHERE task_id = ?1",
            params![task_id.to_string(), now_ms],
        )?;
        enqueue_projection(&tx, task_id, &format!("committing:{attempt_id}"))?;
        tx.commit()?;
        Ok(())
    }
}
