//! 选择器绑定完整原请求和文件证据；按有限原恢复意图枚举，不扫描无限请求库。
use super::{record::Subject, *};
use crate::{content_audit::store as events, mutation, recorded::store as requests};

pub(super) enum Review {
    Mutation(Box<crate::PendingMutation>),
    Restore(Box<requests::Envelope>),
    FileOperation(Box<crate::file_operation::maintenance::Review>),
}
impl Review {
    pub fn subject(&self) -> Result<Subject, BrokerError> {
        Ok(match self {
            Self::FileOperation(r) => Subject::FileOperation { review: r.clone() },
            Self::Mutation(r) => Subject::Mutation {
                operation: Box::new(r.maintenance_operation()),
            },
            Self::Restore(e) => Subject::Restore {
                reference: e.reference()?,
            },
        })
    }
    pub fn target(&self, broker: &FsVaultBroker) -> Result<MaintenanceTarget, BrokerError> {
        match self {
            Self::FileOperation(r) => crate::file_operation::maintenance::target(r),
            Self::Mutation(r) => r.maintenance_target(broker),
            Self::Restore(e) => restore_target(broker, e),
        }
    }
}
pub(super) fn reviews(broker: &FsVaultBroker, limit: usize) -> Result<Vec<Review>, BrokerError> {
    super::limit(limit)?;
    let mut result = Vec::new();
    if mutation::load(broker)?.batch.is_some() {
        return Err(BrokerError::MutationUnresolved);
    }
    if mutation::load(broker)?.file_operation.is_some() {
        if let Some(review) = crate::file_operation::maintenance::capture(broker)? {
            result.push(Review::FileOperation(Box::new(review)));
        }
        return Ok(result);
    }
    if let Some(review) = broker.pending_mutation()? {
        let operation = review.maintenance_operation();
        if mutation::replay::load(
            &broker.data,
            &operation.key_hash,
            broker.writer.binding.pair_id,
        )?
        .is_some()
        {
            // 已有固定原终态只待重开整理，不再形成另一笔维护决定。
            return Ok(result);
        }
        result.push(Review::Mutation(Box::new(review)));
        // 有安装槽时，未安装的其他恢复 Intent 不能由同一个维护动作取消。
        return Ok(result);
    }
    let names = events::entries(broker)?;
    for filename in &names {
        let (envelope, _) = events::lifecycle(broker, filename)?;
        if matches!(envelope.kind, Some(crate::MutationKind::Restore { .. }))
            && events::terminal(broker, &envelope)?.is_none()
            && result.len() < limit
        {
            result.push(Review::Restore(Box::new(envelope)));
        }
    }
    if events::entries(broker)? != names {
        return Err(BrokerError::CaptureChanged);
    }
    broker.writer.check(&broker.root, &broker.data)?;
    Ok(result)
}
pub(super) fn find(
    broker: &FsVaultBroker,
    id: &str,
) -> Result<Option<(Review, MaintenanceTarget)>, BrokerError> {
    for review in reviews(broker, 64)? {
        let target = review.target(broker)?;
        if target.review_id == id {
            return Ok(Some((review, target)));
        }
    }
    Ok(None)
}
fn restore_target(
    broker: &FsVaultBroker,
    e: &requests::Envelope,
) -> Result<MaintenanceTarget, BrokerError> {
    let state = mutation::load(broker)?;
    if state.batch.is_some()
        || state.operation.is_some()
        || state.file_operation.is_some()
        || events::terminal(broker, e)?.is_some()
    {
        return Err(BrokerError::CaptureChanged);
    }
    let encoded = serde_json::to_vec(&(
        "maintenance-restore-v1",
        e,
        state.pair_id,
        state.committed_through,
        state.next_sequence,
    ))
    .map_err(|_| BrokerError::CorruptWriterState)?;
    Ok(MaintenanceTarget {
        review_id: crate::sha256(&encoded),
        request_fingerprint: e.fingerprint.clone(),
        path: e.path.clone(),
        kind: MaintenanceTargetKind::RestoreIntent,
        sequence: None,
        previous_revision: match e.precondition {
            crate::Precondition::Match(r) => Some(r),
            _ => None,
        },
        target_revision: Some(e.target),
        file_operation_id: None,
        secondary: None,
        file_operation: None,
        current: MaintenanceCurrent::NotObserved,
        proposed_available: false,
    })
}
pub(super) fn validate(
    broker: &FsVaultBroker,
    subject: &Subject,
    target: &MaintenanceTarget,
    decision: MaintenanceDecision,
) -> Result<(), BrokerError> {
    if let Subject::FileOperation { review } = subject {
        return crate::file_operation::maintenance::validate(broker, review, target, decision);
    }
    if target.secondary.is_some()
        || target.file_operation.is_some()
        || target.file_operation_id.is_some()
        || matches!(target.current, MaintenanceCurrent::Unobservable { .. })
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let (path, previous, revision, fingerprint, kind, sequence) = match subject {
        Subject::FileOperation { .. } => return Err(BrokerError::CorruptWriterState),
        Subject::Mutation { operation: op } => {
            let mut state = mutation::State::initial(broker.writer.binding.pair_id, false);
            state.format_version = if op.stage.size > crate::writer::disk::CONTENT_LIMIT as u64 {
                4
            } else {
                3
            };
            state.maintenance_enabled = true;
            state.next_sequence = op
                .sequence
                .checked_add(1)
                .ok_or(BrokerError::CorruptWriterState)?;
            state.operation = Some((**op).clone());
            state.validate(state.pair_id)?;
            let kind = match op.phase {
                mutation::record::Phase::Prepared => MaintenanceTargetKind::Prepared,
                mutation::record::Phase::Installing => MaintenanceTargetKind::Installing,
                _ => return Err(BrokerError::CorruptWriterState),
            };
            if matches!(target.current, MaintenanceCurrent::NotObserved) {
                return Err(BrokerError::CorruptWriterState);
            }
            (
                op.path.clone(),
                op.previous_revision,
                op.target_revision,
                op.fingerprint.clone(),
                kind,
                Some(op.sequence),
            )
        }
        Subject::Restore { reference } => {
            let e = requests::load(broker, reference)?;
            if !matches!(e.kind, Some(crate::MutationKind::Restore { .. }))
                || target.current != MaintenanceCurrent::NotObserved
                || target.proposed_available
            {
                return Err(BrokerError::CorruptWriterState);
            }
            let previous = match e.precondition {
                crate::Precondition::Match(r) => Some(r),
                _ => None,
            };
            (
                e.path,
                previous,
                e.target,
                e.fingerprint,
                MaintenanceTargetKind::RestoreIntent,
                None,
            )
        }
    };
    if path != target.path
        || previous != target.previous_revision
        || Some(revision) != target.target_revision
        || fingerprint != target.request_fingerprint
        || kind != target.kind
        || sequence != target.sequence
        || !allowed(kind, decision)
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
pub(super) fn allowed(kind: MaintenanceTargetKind, decision: MaintenanceDecision) -> bool {
    matches!(
        (kind, decision),
        (
            MaintenanceTargetKind::Prepared,
            MaintenanceDecision::CancelPrepared
        ) | (
            MaintenanceTargetKind::Installing,
            MaintenanceDecision::PreserveInstalling
        ) | (
            MaintenanceTargetKind::RestoreIntent,
            MaintenanceDecision::CancelRestoreIntent
        )
    )
}
