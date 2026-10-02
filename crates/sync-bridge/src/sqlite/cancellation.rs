//! 未发动作的新鲜条件已变：原子消费预约并取消，不向调用方发放执行能力。
use super::{BridgeError, BridgeState, dispatch::checked_action, model::*, receipts};
use crate::{ActionKey, Side};
use std::time::Instant;

impl BridgeState {
    /// 仅取消条件已变、Intent 已确认且从未发出的动作。
    /// 返回 true 表示取消已持久化；false 表示条件未变，应继续正常 issue。
    /// 已发结果未知、证据过期/绑定不符及审计未确认均拒绝，不推断终态。
    pub fn cancel_changed(&mut self, key: &ActionKey, check: ExecutionCheck) -> Result<bool> {
        if check.process_epoch != self.epoch || Instant::now() >= check.expires_at {
            return Err(BridgeError::StaleObservation);
        }
        let (index, changed) = checked_action(self.read()?, key, &check)?;
        let p = self.read()?.pending.as_ref().ok_or(BridgeError::NotFound)?;
        if p.actions[index].issued {
            return Err(BridgeError::NotSettled);
        }
        if !changed {
            return Ok(false);
        }
        let deadline = check.expires_at;
        self.edit_guarded(
            Edit::Action {
                sequence: key.plan.sequence,
                index,
            },
            None,
            || {
                if Instant::now() < deadline {
                    Ok(())
                } else {
                    Err(BridgeError::StaleObservation)
                }
            },
            |s| {
                let (index, changed) = checked_action(s, key, &check)?;
                let p = s.pending.as_mut().ok_or(BridgeError::NotFound)?;
                if p.actions[index].issued || !changed {
                    return Err(BridgeError::NotSettled);
                }
                let action = &receipts::plan(p)?.actions()[index];
                let request_digest = receipts::request_hash(p, index)?;
                let receipt = EffectReceipt {
                    external_key: p.actions[index].key.clone(),
                    request_digest,
                    backend: match key.target {
                        Side::Canonical => Backend::Broker {
                            canonical_root: check.binding.canonical_root_token,
                        },
                        Side::Mirror => Backend::Mirror {
                            mirror_root: check.binding.mirror_root_token,
                        },
                    },
                    outcome: EffectOutcome::NoEffect {
                        path: key.path.as_str().to_owned(),
                        expected_revision: receipts::expected(action),
                        reason: NoEffectReason::CancelledBeforeInstall,
                        terminal_id: format!(
                            "bridge-unissued-cancel-{}",
                            receipts::hex(&request_digest)
                        ),
                    },
                };
                receipts::validate(p, index, &receipt)?;
                // 兼容现有持久模型：同一事务消费预约并保存取消票据，绝不暴露 Issued Unknown。
                // 此处不返回 PendingAction，也不向任何文件写入后端发放原键。
                p.actions[index].issued = true;
                p.actions[index].receipt = Some(receipt);
                Ok(())
            },
        )?;
        if Instant::now() >= deadline {
            return Err(BridgeError::StaleObservation);
        }
        Ok(true)
    }
}
