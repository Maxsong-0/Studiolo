//! 生产者答「该不该抽查仓库」、收抽查结果、给健康页抽查事实；节奏和组号只看账本（`checks.rs`）。
//!
//! 派检查和收结果都拿着状态锁，和补「没做完」、排期串行：同一次检查不会一边被补记、一边落 worker 的结果。
use super::{BackupProducer, TARGET, now_millis};
use crate::backup::check_evidence::RepositoryChecks;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
};
use studiolo_backup::worker::{
    audit::{QueueAnswer, RepositoryCheckReport},
    repository_check::{
        CheckAssignment, CheckEvent, CheckKey, check_abandoned, check_intent, read_check_event,
    },
};

impl BackupProducer {
    /// 空闲的 worker 问该不该抽查：有作业没收口、在封存就先做作业；先补中途没了的原意图，
    /// 再按账本定节奏和组号，到期就把原意图落账，回它的回执。
    pub(super) fn repository_check(&self) -> Result<QueueAnswer, AuditError> {
        let mut state = self.state.lock().map_err(|_| AuditError::Unavailable)?;
        if state.busy {
            return Ok(QueueAnswer::Outstanding);
        }
        self.refresh(&mut state.queue)?;
        if state.queue.outstanding() {
            return Ok(QueueAnswer::Outstanding);
        }
        let now = now_millis()?;
        let abandoned = state.queue.checks().abandoned(now);
        for key in &abandoned {
            self.ledger
                .append(check_abandoned(key).map_err(|_| AuditError::Serialization)?)?;
        }
        if !abandoned.is_empty() {
            self.refresh(&mut state.queue)?;
        }
        let checks = state.queue.checks();
        if checks.running(now).is_some() {
            return Ok(QueueAnswer::Outstanding);
        }
        let Some((bucket, purpose)) = checks.due(now) else {
            return Ok(QueueAnswer::NotDue);
        };
        let sequence = checks.next_sequence().ok_or(AuditError::Unavailable)?;
        let assignment = CheckAssignment {
            key: CheckKey { sequence, bucket },
            purpose,
        };
        let expected = check_intent(&assignment, TARGET).map_err(|_| AuditError::Serialization)?;
        let record = self.ledger.append(expected.clone())?;
        record.verify_integrity()?;
        if record.event() != &expected {
            return Err(AuditError::CorruptStore);
        }
        self.refresh(&mut state.queue)?;
        Ok(QueueAnswer::CheckAssigned(Box::new(record)))
    }

    /// worker 交来一次抽查的结果：账本里要有同一检查键、本目标的原意图，才追加这份上报导出的唯一
    /// 结果事件。同一份上报重放回原回执；同一检查已经有别的结果（例如补过「没做完」）就拒绝。
    pub(super) fn report_check(
        &self,
        request: &RepositoryCheckReport,
    ) -> Result<Vec<u8>, AuditError> {
        let event = request.event().map_err(|_| AuditError::Serialization)?;
        let mut state = self.state.lock().map_err(|_| AuditError::Unavailable)?;
        let intent = self
            .check_intent_record(&event)?
            .ok_or(AuditError::Unavailable)?;
        let same = matches!(
            read_check_event(intent.event()),
            Some(CheckEvent::Intent { assignment, target })
                if assignment.key == *request.key() && target == TARGET
        );
        if !same {
            return Err(AuditError::CorruptStore);
        }
        let record = self.ledger.append(event)?;
        // 健康页和下一次排期要读到这条结果；这一次没读完不影响已经落账的回执，下次接着读。
        let _ = self.refresh(&mut state.queue);
        drop(state);
        request
            .encode_reply(&record)
            .map_err(|_| AuditError::CorruptStore)
    }

    /// 健康页的抽查事实：和排期读同一份累计视图，读不完就这一次不给结论。
    pub(super) fn repository_checks_at(&self, now_ms: i64) -> Option<RepositoryChecks> {
        let mut state = self.state.lock().ok()?;
        self.refresh(&mut state.queue).ok()?;
        let queue = &state.queue;
        let anchor = [
            self.serving_since_ms,
            queue.first_attempt_ms(),
            queue.checks().first_intent_ms(),
        ]
        .into_iter()
        .flatten()
        .min();
        Some(RepositoryChecks::from_view(now_ms, anchor, queue.checks()))
    }

    /// 同一检查键的原意图回执；意图和结果共用事件键。
    fn check_intent_record(&self, outcome: &AuditEvent) -> Result<Option<AuditRecord>, AuditError> {
        let mut records = self.ledger.query(
            &AuditFilter::default()
                .with_event_id(outcome.event_id().clone())
                .with_phase(AuditPhase::Intent)
                .with_limit(2),
        )?;
        if records.len() > 1 {
            return Err(AuditError::CorruptStore);
        }
        let record = records.pop();
        if let Some(record) = &record {
            record.verify_integrity()?;
        }
        Ok(record)
    }
}
