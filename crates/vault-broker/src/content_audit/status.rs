//! 未确认恢复状态从真实生命周期和永久终态派生。
use super::{ContentAuditOutcome, ContentAuditStatus, store as events};
use crate::{BrokerError, FsVaultBroker, MutationKind};
impl FsVaultBroker {
    /// 从真实终态判定恢复是否仍可能已安装，普通拒绝/取消不会冻结后续普通保存。
    pub fn content_audit_status(&self) -> Result<ContentAuditStatus, BrokerError> {
        let names = events::entries(self)?;
        let mut status = ContentAuditStatus {
            pending: 0,
            restore_unconfirmed: false,
        };
        for filename in &names {
            let (envelope, intent) = events::lifecycle(self, filename)?;
            if !events::acknowledged(self, &envelope, &intent)? {
                status.pending += 1;
            }
            let outcome = events::outcome_row(self, &envelope)?;
            let outcome_ack = outcome
                .as_ref()
                .map(|row| events::acknowledged(self, &envelope, row))
                .transpose()?
                .unwrap_or(false);
            if outcome.is_some() && !outcome_ack {
                status.pending += 1;
            }
            let harmless = outcome.as_ref().is_some_and(|row| {
                matches!(
                    row.event.outcome,
                    Some(ContentAuditOutcome::Rejected | ContentAuditOutcome::Cancelled)
                )
            });
            if matches!(envelope.kind, Some(MutationKind::Restore { .. }))
                && !harmless
                && !outcome_ack
            {
                status.restore_unconfirmed = true;
            }
        }
        if events::entries(self)? != names {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(status)
    }
}
