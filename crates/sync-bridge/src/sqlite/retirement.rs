//! 清理需要完整终态和独立真实审计 ACK，不提供无审计逐出。
use super::{BridgeError, BridgeState, audit, model::*, receipts};

impl BridgeState {
    /// 清理候选只冻结完整已审计 last；ACK 前不删除、不推进 floor。
    pub fn prepare_retirement(&mut self, expected_version: u64) -> Result<()> {
        self.edit(Edit::Retirement, Some(expected_version), |s| {
            if s.fuse.is_some() {
                return Err(BridgeError::Fused);
            }
            let last = s.last.as_ref().ok_or(BridgeError::NotFound)?;
            if !receipts::acknowledged(last) {
                return Err(BridgeError::NotSettled);
            }
            let ticket = Retirement {
                old_sequence: last.input.plan_sequence,
                old_digest: audit::terminal_digest(last)?,
                old_floor: s.floor,
                ack: None,
            };
            if let Some(prior) = &s.retirement {
                if prior.old_sequence == ticket.old_sequence
                    && prior.old_digest == ticket.old_digest
                    && prior.old_floor == ticket.old_floor
                {
                    return Ok(());
                }
                return Err(BridgeError::RetryMismatch);
            }
            s.retirement = Some(ticket);
            Ok(())
        })
    }
}
