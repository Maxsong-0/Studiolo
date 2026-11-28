//! 受保护的黑板效果发生之前先写审计。重放共用稳定的请求摘要。
//!
//! 审计身份就是请求摘要（`board:<动作>:<摘要>`），账本按「事件号＋阶段」去重：同一个请求的 Intent、
//! Outcome 各只有一条，记下的结论再也改不了。所以一次请求的结果分三态（BUG-060）：
//!
//! - 成功：记 `succeeded`。
//! - 确定拒绝：引擎依据读得到的状态说了不，记 `rejected`；之后同一请求一律照样拒绝、什么都不做，
//!   条件变了也不会「先拒后成」，账本里一个请求只有一个结论。要重来就换一个新请求。
//! - 没下结论：控制库忙或出错、Broker 或账本这一刻不可用、提交停在预留、投影没做完等，不记 Outcome，
//!   只留意图；同一请求可以重试，由下一次下了结论的那次补上。以前一律记 `rejected`：重试成功时
//!   和它冲突，动作做了（号发出去了、任务建了）、调用方却拿到错误，之后同一请求永远失败。
//!
//! 结论只能由结果推出来（[`BoardVerdict::of`]），调用处写不出「任何错误都算拒绝」。
use super::*;
use crate::board_types::{contains_conflict_markers, sha256_hex};

/// 一个黑板请求在账本里的身份：[`PersistentTaskEngine::board_intent`] 记下意图时发出，
/// 交给 [`PersistentTaskEngine::board_outcome`] 收尾。
#[must_use = "记了意图就要用 board_outcome 收尾"]
pub(super) struct BoardAudit {
    action: String,
    id: AuditEventId,
    digest: String,
    /// 同一请求之前已经成功过，这次是重放。
    succeeded_before: bool,
}

impl BoardAudit {
    fn new<T: Serialize>(action: &str, request: &T) -> Result<Self, TaskEngineError> {
        let digest = sha256_hex(&serde_json::to_vec(request)?);
        Ok(Self {
            id: AuditEventId::parse(format!("board:{action}:{digest}")).map_err(map_audit)?,
            action: action.to_owned(),
            digest,
            succeeded_before: false,
        })
    }
}

/// 一次请求的结论。只能由结果推出来：[`Self::of`]，预演过的计划用 [`Self::of_plan`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BoardVerdict(Verdict);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Succeeded,
    Refused,
    Unconcluded,
}

impl BoardVerdict {
    pub(super) fn of<T>(result: &Result<T, TaskEngineError>) -> Self {
        Self(match result {
            Ok(_) => Verdict::Succeeded,
            Err(error) => verdict_of_error(error),
        })
    }

    /// 预演过的计划：执行时库已经变了、整轮什么都没做（`Ok(None)`），是确定的「没做」，记成拒绝。
    pub(super) fn of_plan<T>(result: &Result<Option<T>, TaskEngineError>) -> Self {
        Self(match result {
            Ok(Some(_)) => Verdict::Succeeded,
            Ok(None) => Verdict::Refused,
            Err(error) => verdict_of_error(error),
        })
    }
}

/// 一种错误算确定拒绝还是没下结论。每种都列出来、不用通配：新加一种错误时这里编译不过，逼着先想清楚它归哪边。
/// 拿不准的归「没下结论」：它最多让账本里留一条没有结论的意图，不会记下假的拒绝，也不会挡住之后的成功。
fn verdict_of_error(error: &TaskEngineError) -> Verdict {
    use TaskEngineError as E;
    match error {
        // 引擎读到了权威状态、按规则说了不：请求本身不合法、对象不在、状态不允许、fence 过期、已有终态赢家、
        // 同一身份换了内容、路径被占、输入要重新确认、runner 引用不对，或者这个请求之前就被拒过。
        E::InvalidTaskSpec(_)
        | E::NotFound(_)
        | E::InvalidTransition { .. }
        | E::StaleFence
        | E::AlreadyTerminal
        | E::IdempotencyMismatch
        | E::PathsHeld { .. }
        | E::NeedsReconfirmation(_)
        | E::InvalidRunnerRef
        | E::RequestAlreadyRejected => Verdict::Refused,
        E::ControlDb(error) => verdict_of_db(error),
        // 这一刻做不成，或者不知道做没做成：账本、Broker 不可用或出错，提交停在预留、留待对账前滚，
        // 投影没做完（权威状态可能已经提交），编码出错，或者核对出库里的东西不一致（要人修，不是请求的错）。
        E::AuditUnavailable
        | E::Audit(_)
        | E::Broker(_)
        | E::CommitFailed(_)
        | E::Json(_)
        | E::Integrity(_)
        | E::Projection(_)
        | E::ProjectionIncomplete { .. } => Verdict::Unconcluded,
    }
}

/// 没经 [`map_db`] 归类、原样包着的控制库错误：规则拒绝照样是确定拒绝，SQLite 本身的错误没下结论。
fn verdict_of_db(error: &DbError) -> Verdict {
    match error {
        DbError::NotFound(_)
        | DbError::InvalidTransition { .. }
        | DbError::AlreadyTerminal { .. }
        | DbError::FenceMismatch
        | DbError::PathsHeld { .. }
        | DbError::BoardRejected(_)
        | DbError::IdempotencyMismatch => Verdict::Refused,
        // SQLite 忙、锁住、IO 出错都在这里：事务已回滚，同一请求重试可以成功。
        DbError::Sqlite(_)
        | DbError::Json(_)
        | DbError::VaultBindingMismatch { .. }
        | DbError::UnsupportedSchema { .. }
        | DbError::Corrupt(_) => Verdict::Unconcluded,
    }
}

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 受保护的效果之前记意图。同一请求之前已被确定拒绝的，这次照样拒绝、什么都不做。
    pub(super) fn board_intent<T: Serialize>(
        &mut self,
        action: &str,
        request: &T,
    ) -> Result<BoardAudit, TaskEngineError> {
        let mut audit = BoardAudit::new(action, request)?;
        self.append_board(&audit, AuditPhase::Intent, None)?;
        match self.recorded_board_outcome(&audit)? {
            Some(false) => Err(TaskEngineError::RequestAlreadyRejected),
            Some(true) => {
                audit.succeeded_before = true;
                Ok(audit)
            }
            None => Ok(audit),
        }
    }

    /// 按结论收尾：成功先补投运行时结果再记 `succeeded`；确定拒绝记 `rejected`；没下结论不记。
    pub(super) fn board_outcome(
        &mut self,
        audit: BoardAudit,
        verdict: BoardVerdict,
    ) -> Result<(), TaskEngineError> {
        match verdict.0 {
            Verdict::Succeeded => {
                self.deliver_runtime_audits()?;
                self.append_board(&audit, AuditPhase::Outcome, Some(true))
            }
            // 同一请求之前已成功、这次重放没成：账本里的结论不改（改了就和那一条冲突），调用方拿到这次的错误。
            Verdict::Refused if audit.succeeded_before => Ok(()),
            Verdict::Refused => self.append_board(&audit, AuditPhase::Outcome, Some(false)),
            Verdict::Unconcluded => Ok(()),
        }
    }

    /// 不对应哪个请求的「待处理」标记（只有 Outcome，记成没成）：同一个对象只记一次，账本按幂等键重放。
    pub(super) fn board_pending_marker<T: Serialize>(
        &mut self,
        action: &str,
        subject: &T,
    ) -> Result<(), TaskEngineError> {
        let audit = BoardAudit::new(action, subject)?;
        self.append_board(&audit, AuditPhase::Outcome, Some(false))
    }

    /// 账本里这个请求已有的结论：`Some(true)` 成功过，`Some(false)` 被拒过，`None` 还没有结论。
    fn recorded_board_outcome(&self, audit: &BoardAudit) -> Result<Option<bool>, TaskEngineError> {
        let records = self
            .audit_ledger
            .query(
                &AuditFilter::default()
                    .with_event_id(audit.id.clone())
                    .with_phase(AuditPhase::Outcome)
                    .with_limit(1),
            )
            .map_err(map_audit)?;
        let Some(record) = records.first() else {
            return Ok(None);
        };
        match record.event().payload().as_value().get("status") {
            Some(serde_json::Value::String(status)) if status == "succeeded" => Ok(Some(true)),
            Some(serde_json::Value::String(status)) if status == "rejected" => Ok(Some(false)),
            _ => Err(TaskEngineError::Integrity(
                "board outcome has no known status".into(),
            )),
        }
    }

    /// 载荷和以前一字不差：已经落账的请求重放时才不会和旧记录冲突。
    fn append_board(
        &mut self,
        audit: &BoardAudit,
        phase: AuditPhase,
        success: Option<bool>,
    ) -> Result<(), TaskEngineError> {
        let payload = if let Some(success) = success {
            json!({"request_hash":audit.digest,"status":if success {"succeeded"}else{"rejected"}})
        } else {
            json!({"request_hash":audit.digest})
        };
        self.audit_ledger
            .append(AuditEvent::new(
                audit.id.clone(),
                phase,
                &audit.action,
                SanitizedPayload::try_from(payload).map_err(map_audit)?,
            ))
            .map_err(map_audit)?;
        Ok(())
    }

    /// 账本中断或进程重启后，补投只含元数据的权威结果。
    pub fn deliver_runtime_audits(&mut self) -> Result<usize, TaskEngineError> {
        let events = self.control_db.pending_runtime_audits().map_err(map_db)?;
        for event in &events {
            self.audit_ledger
                .append(event.audit_event().map_err(map_db)?)
                .map_err(map_audit)?;
            self.control_db
                .acknowledge_runtime_audit(&event.event_id)
                .map_err(map_db)?;
        }
        Ok(events.len())
    }

    pub(super) fn verify_artifact_pointer(
        &self,
        pointer: &ArtifactPointer,
    ) -> Result<(), TaskEngineError> {
        let object = self
            .broker
            .read(&pointer.path)
            .map_err(TaskEngineError::Broker)?;
        if object.revision.to_string() != pointer.revision
            || sha256_hex(&object.content) != pointer.sha256.to_ascii_lowercase()
        {
            return Err(TaskEngineError::Integrity(
                "artifact pointer does not match content-plane bytes".into(),
            ));
        }
        if contains_conflict_markers(&object.content) {
            return Err(TaskEngineError::InvalidTaskSpec(
                "artifact contains unresolved conflict markers",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiolo_audit_ledger::MemoryAuditLedger;
    use studiolo_core::VaultUuid;
    use studiolo_vault_broker::MemoryVaultBroker;

    type Engine = PersistentTaskEngine<MemoryVaultBroker, MemoryAuditLedger>;

    fn engine() -> Engine {
        let vault = VaultUuid::new();
        Engine::new(
            ControlDb::open_in_memory(vault).unwrap(),
            MemoryVaultBroker::with_uuid(vault),
            MemoryAuditLedger::new(),
        )
    }

    fn json_error() -> serde_json::Error {
        serde_json::from_str::<u8>("not json").unwrap_err()
    }

    fn of(error: TaskEngineError) -> Verdict {
        BoardVerdict::of::<()>(&Err(error)).0
    }

    #[test]
    fn every_error_is_either_a_definite_refusal_or_unconcluded() {
        use TaskEngineError as E;
        let refused = [
            E::InvalidTaskSpec("bad"),
            E::NotFound("task"),
            E::InvalidTransition {
                from: "running".into(),
                operation: "claim",
            },
            E::StaleFence,
            E::AlreadyTerminal,
            E::IdempotencyMismatch,
            E::PathsHeld {
                by_instance: "other".into(),
                paths: vec!["src".into()],
            },
            E::NeedsReconfirmation(TaskId::new()),
            E::InvalidRunnerRef,
            E::RequestAlreadyRejected,
            E::ControlDb(DbError::NotFound("task")),
            E::ControlDb(DbError::InvalidTransition {
                from: "running".into(),
                operation: "claim",
            }),
            E::ControlDb(DbError::AlreadyTerminal {
                phase: AttemptPhase::Completed,
                operation: "receipt",
            }),
            E::ControlDb(DbError::FenceMismatch),
            E::ControlDb(DbError::PathsHeld {
                by_instance: "other".into(),
                paths: vec![],
            }),
            E::ControlDb(DbError::BoardRejected(
                "parent must be unclaimed before delegation",
            )),
            E::ControlDb(DbError::IdempotencyMismatch),
        ];
        for error in refused {
            let shown = format!("{error:?}");
            assert_eq!(of(error), Verdict::Refused, "{shown}");
        }
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        );
        let unconcluded = [
            E::ControlDb(DbError::Sqlite(busy)),
            E::ControlDb(DbError::Sqlite(rusqlite::Error::QueryReturnedNoRows)),
            E::ControlDb(DbError::Json(json_error())),
            E::ControlDb(DbError::VaultBindingMismatch {
                expected: VaultUuid::new(),
                actual: VaultUuid::new(),
            }),
            E::ControlDb(DbError::UnsupportedSchema {
                found: 99,
                supported: 1,
            }),
            E::ControlDb(DbError::Corrupt("bad row".into())),
            E::AuditUnavailable,
            E::Audit(AuditError::Serialization),
            E::Broker(BrokerError::WriterBusy),
            E::CommitFailed(BrokerError::WriterBusy),
            E::Json(json_error()),
            E::Integrity("snapshot differs".into()),
            E::Projection("page changed".into()),
            E::ProjectionIncomplete {
                delivered: 0,
                failures: vec![],
            },
        ];
        for error in unconcluded {
            let shown = format!("{error:?}");
            assert_eq!(of(error), Verdict::Unconcluded, "{shown}");
        }
        assert_eq!(
            BoardVerdict::of(&Ok::<_, TaskEngineError>(1)).0,
            Verdict::Succeeded
        );
    }

    #[test]
    fn a_plan_that_went_stale_is_refused_and_a_busy_one_is_unconcluded() {
        assert_eq!(
            BoardVerdict::of_plan(&Ok::<_, TaskEngineError>(Some(1))).0,
            Verdict::Succeeded
        );
        assert_eq!(
            BoardVerdict::of_plan(&Ok::<Option<u8>, TaskEngineError>(None)).0,
            Verdict::Refused
        );
        assert_eq!(
            BoardVerdict::of_plan::<u8>(&Err(TaskEngineError::AuditUnavailable)).0,
            Verdict::Unconcluded
        );
        assert_eq!(
            BoardVerdict::of_plan::<u8>(&Err(TaskEngineError::StaleFence)).0,
            Verdict::Refused
        );
    }

    fn outcomes(engine: &Engine) -> Vec<String> {
        engine
            .audit_ledger()
            .query(&AuditFilter::default().with_phase(AuditPhase::Outcome))
            .unwrap()
            .iter()
            .map(|record| record.event().payload().as_value()["status"].to_string())
            .collect()
    }

    #[test]
    fn a_refused_replay_of_a_request_that_already_succeeded_keeps_the_recorded_success() {
        let mut engine = engine();
        let audit = engine.board_intent("task.test", &"r1").unwrap();
        assert!(!audit.succeeded_before);
        engine
            .board_outcome(audit, BoardVerdict::of(&Ok::<_, TaskEngineError>(())))
            .unwrap();
        let replay = engine.board_intent("task.test", &"r1").unwrap();
        assert!(replay.succeeded_before);
        engine
            .board_outcome(
                replay,
                BoardVerdict::of::<()>(&Err(TaskEngineError::StaleFence)),
            )
            .expect("账本里的成功不改，也不和它冲突");
        assert_eq!(outcomes(&engine), ["\"succeeded\""]);
    }

    #[test]
    fn a_refused_request_is_refused_again_without_reaching_the_effect() {
        let mut engine = engine();
        let audit = engine.board_intent("task.test", &"r1").unwrap();
        engine
            .board_outcome(
                audit,
                BoardVerdict::of::<()>(&Err(TaskEngineError::InvalidTaskSpec("bad"))),
            )
            .unwrap();
        assert!(matches!(
            engine.board_intent("task.test", &"r1"),
            Err(TaskEngineError::RequestAlreadyRejected)
        ));
        let other = engine.board_intent("task.test", &"r2").unwrap();
        assert!(!other.succeeded_before, "换一个请求不受影响");
        engine
            .board_outcome(
                other,
                BoardVerdict::of::<()>(&Err(TaskEngineError::AuditUnavailable)),
            )
            .unwrap();
        assert_eq!(outcomes(&engine), ["\"rejected\""], "没下结论的不记");
    }

    #[test]
    fn an_outcome_with_an_unknown_status_fails_closed() {
        let mut engine = engine();
        let audit = BoardAudit::new("task.test", &"r1").unwrap();
        engine
            .audit_ledger()
            .append(AuditEvent::new(
                audit.id.clone(),
                AuditPhase::Outcome,
                "task.test",
                SanitizedPayload::try_from(json!({"request_hash":audit.digest,"status":"maybe"}))
                    .unwrap(),
            ))
            .unwrap();
        assert!(matches!(
            engine.board_intent("task.test", &"r1"),
            Err(TaskEngineError::Integrity(_))
        ));
    }

    #[test]
    fn payloads_stay_byte_identical_to_records_written_before_bug_060() {
        // 修复前落账的请求重放时要和旧记录逐字一致，否则会撞账本：载荷只能是这两种形状。
        let mut engine = engine();
        let audit = engine.board_intent("task.test", &"r1").unwrap();
        let digest = audit.digest.clone();
        engine
            .board_outcome(audit, BoardVerdict::of(&Ok::<_, TaskEngineError>(())))
            .unwrap();
        let records = engine
            .audit_ledger()
            .query(&AuditFilter::default())
            .unwrap();
        assert_eq!(
            records[0].event().event_id().as_str(),
            format!("board:task.test:{digest}")
        );
        assert_eq!(
            records[0].event().payload().as_value(),
            &json!({"request_hash":digest})
        );
        assert_eq!(
            records[1].event().payload().as_value(),
            &json!({"request_hash":digest,"status":"succeeded"})
        );
        assert_eq!(digest, sha256_hex(&serde_json::to_vec(&"r1").unwrap()));
    }
}
