//! 先封存双方、再写不可变终态、最后解锁；任一中断可在重开时只续已持久决策。

use super::{
    PendingDecision, PendingMutation, ReconciliationOutcome, ReconciliationReceipt, evidence,
    observation,
};
use crate::mutation::{
    self,
    record::{Outcome, Phase, Terminal},
};
use crate::writer::{directory, disk};
use crate::{BrokerError, FsVaultBroker};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

impl FsVaultBroker {
    /// 消费同实例最新审阅；调用方须在 owner 内最终重验主人权限和审计意图。
    /// 此方法不写 canonical，不证明原安装结果，也不能协调 legacy/损坏存储。
    pub fn reconcile_pending(
        &mut self,
        review: &PendingMutation,
        decision: PendingDecision,
    ) -> Result<ReconciliationReceipt, BrokerError> {
        crate::maintenance::ensure_idle(self)?;
        self.reconcile_maintenance_pending(review, decision)
    }
    pub(crate) fn reconcile_maintenance_pending(
        &mut self,
        review: &PendingMutation,
        decision: PendingDecision,
    ) -> Result<ReconciliationReceipt, BrokerError> {
        self.reconcile_proposal(review, |broker| match decision {
            PendingDecision::CancelPrepared => Ok(None),
            PendingDecision::PreserveCurrent { original_content } => {
                super::proposed::Proposed::buffered(broker, review.operation(), original_content)
                    .map(Some)
            }
        })
    }
    pub(crate) fn reconcile_maintenance_proposed(
        &mut self,
        review: &PendingMutation,
        proposed: Option<super::proposed::Proposed>,
    ) -> Result<ReconciliationReceipt, BrokerError> {
        self.reconcile_proposal(review, |_| Ok(proposed))
    }

    fn reconcile_proposal(
        &mut self,
        review: &PendingMutation,
        prepare: impl FnOnce(&Self) -> Result<Option<super::proposed::Proposed>, BrokerError>,
    ) -> Result<ReconciliationReceipt, BrokerError> {
        observation::check(self, review)?;
        let next = review
            .ticket
            .checked_add(1)
            .ok_or(BrokerError::WriterLimitExceeded)?;
        self.recovery_review
            .compare_exchange(review.ticket, next, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| BrokerError::CaptureChanged)?;
        // 先消费票据；内部复核仍使用本次独占的新代次，失败后也不得重用旧票据。
        let proposed = prepare(self)?;
        let op = review.operation();
        let (outcome, evidence_id) = match (op.phase, proposed) {
            (Phase::Prepared, None) => {
                mutation::install::verify_stage(self, op)?;
                (ReconciliationOutcome::Cancelled, None)
            }
            (Phase::Installing, Some(proposed)) => {
                validate_optional_stage(self, op)?;
                (
                    ReconciliationOutcome::Indeterminate,
                    Some(evidence::preserve(self, review, proposed)?),
                )
            }
            _ => return Err(BrokerError::MutationUnresolved),
        };
        mutation::fault(self, 7)?;
        // 证据 IO 期间若目标或日志变化，本次没有终态，仍保持 pending。
        if observation::capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)?
            != review.journal
            || observation::current(self, &op.path)? != review.current
            || observation::capture(
                &self.root,
                &format!(".workbench/staging/{}", op.stage.name),
                crate::blob::OBJECT_LIMIT as usize,
            )? != review.stage
        {
            return Err(BrokerError::CaptureChanged);
        }
        let current = review.current_revision();
        let terminal = Terminal {
            recording: op.recording.clone(),
            format_version: if op.recording.is_some() { 4 } else { 3 },
            pair_id: review.state.pair_id,
            sequence: None,
            fingerprint: op.fingerprint.clone(),
            expected_revision: op.target_revision,
            outcome: match &evidence_id {
                None => Outcome::Cancelled {
                    path: op.path.clone(),
                    precondition: op.precondition,
                },
                Some(id) => Outcome::Indeterminate {
                    path: op.path.clone(),
                    precondition: op.precondition,
                    evidence_id: id.clone(),
                    current_revision: current,
                },
            },
        };
        // 原提交水位不变，但任何此前捕获都不再有资格发布。
        self.capture_instance = Arc::new(());
        mutation::save_terminal(self, &op.key_hash, &terminal)?;
        mutation::fault(self, 8)?;
        let mut state = mutation::load(self)?;
        if serde_json::to_vec(&state).map_err(|_| BrokerError::CorruptWriterState)?
            != serde_json::to_vec(&review.state).map_err(|_| BrokerError::CorruptWriterState)?
        {
            return Err(BrokerError::CaptureChanged);
        }
        mutation::finish(self, &mut state)?;
        Ok(ReconciliationReceipt {
            path: op.path.clone(),
            sequence: op.sequence,
            outcome,
            evidence_id,
            current_revision: current,
            committed_through: state.committed_through,
        })
    }
}

pub(super) fn validate_optional_stage(
    broker: &FsVaultBroker,
    op: &mutation::record::Operation,
) -> Result<(), BrokerError> {
    let dir = directory(&broker.root, ".workbench/staging")?;
    if crate::blob::store::exists(&dir, &op.stage.name)? {
        mutation::install::verify_stage(broker, op)?;
    }
    Ok(())
}

pub(super) fn discard_optional_stage(
    broker: &FsVaultBroker,
    op: &mutation::record::Operation,
) -> Result<(), BrokerError> {
    validate_optional_stage(broker, op)?;
    let dir = directory(&broker.root, ".workbench/staging")?;
    if crate::blob::store::exists(&dir, &op.stage.name)? {
        dir.remove_file(&op.stage.name)
            .map_err(|e| BrokerError::io("remove reconciled stage", e))?;
        FsVaultBroker::sync_directory(&dir, Path::new("."))?;
    }
    broker.writer.check(&broker.root, &broker.data)
}
