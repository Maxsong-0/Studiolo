//! 执行前真实 Intent ACK 与新观察门禁；原键未知状态不可被相同内容替代。
use super::{
    BridgeError, BridgeState,
    lookup::{binding_key, select},
    model::*,
    receipts,
};
use crate::{ActionKey, Side};

impl BridgeState {
    /// 真实 Intent ACK 与新鲜观察通过后持久化 Issued，再返回原键执行数据。
    pub fn issue(&mut self, key: &ActionKey, check: ExecutionCheck) -> Result<PendingAction> {
        binding_key(self.read()?, key)?;
        if check.process_epoch != self.epoch || std::time::Instant::now() >= check.expires_at {
            return Err(BridgeError::StaleObservation);
        }
        let index = receipts::index(
            self.read()?.pending.as_ref().ok_or(BridgeError::NotFound)?,
            key,
        )?;
        let deadline = check.expires_at;
        let result = self.edit_guarded(
            Edit::Action {
                sequence: key.plan.sequence,
                index,
            },
            None,
            || {
                if std::time::Instant::now() < deadline {
                    Ok(())
                } else {
                    Err(BridgeError::StaleObservation)
                }
            },
            |s| {
                binding_key(s, key)?;
                if s.fuse.is_some() {
                    return Err(BridgeError::Fused);
                }
                let p = s.pending.as_mut().ok_or(BridgeError::NotFound)?;
                let index = receipts::index(p, key)?;
                let action = receipts::plan(p)?.actions()[index].clone();
                if p.actions[index].receipt.is_some() {
                    return Err(BridgeError::NotSettled);
                }
                if p.actions[index].intent_ack.is_none() {
                    return Err(BridgeError::IntentRequired);
                }
                if check.binding != p.input.scope.binding {
                    return Err(BridgeError::BindingMismatch);
                }
                if check.canonical_observation <= p.input.canonical.sequence
                    || check.mirror_observation <= p.input.mirror.sequence
                    || check.canonical_observation > i64::MAX as u64
                    || check.mirror_observation > i64::MAX as u64
                {
                    return Err(BridgeError::StaleObservation);
                }
                let path = &action.key.path;
                let (source, target) = match action.key.target {
                    Side::Canonical => (
                        &p.input.mirror.entries[path],
                        &p.input.canonical.entries[path],
                    ),
                    Side::Mirror => (
                        &p.input.canonical.entries[path],
                        &p.input.mirror.entries[path],
                    ),
                };
                if source != &check.source || target != &check.target {
                    return Err(BridgeError::StaleObservation);
                }
                // 同 Issued 原键可再次返回，但不能清空未知或更换请求；安装后条件已变应先查询原回执。
                p.actions[index].issued = true;
                Ok(PendingAction {
                    action,
                    external_key: p.actions[index].key.clone(),
                    request_digest: receipts::request_hash(p, index)?,
                })
            },
        )?;
        if std::time::Instant::now() >= deadline {
            return Err(BridgeError::StaleObservation);
        }
        Ok(result)
    }
    /// 可信执行适配器提供真实回执；即使观察过期或已重开，原 key 的核对仍可进行。
    pub fn record_outcome(&mut self, key: &ActionKey, receipt: EffectReceipt) -> Result<()> {
        binding_key(self.read()?, key)?;
        let index = receipts::index(select(self.read()?, key)?, key)?;
        self.edit(
            Edit::Action {
                sequence: key.plan.sequence,
                index,
            },
            None,
            |s| {
                binding_key(s, key)?;
                let p = select(s, key)?;
                let index = receipts::index(p, key)?;
                receipts::validate(p, index, &receipt)?;
                if let Some(prior) = &p.actions[index].receipt {
                    return if prior == &receipt {
                        Ok(())
                    } else {
                        Err(BridgeError::OutcomeMismatch)
                    };
                }
                let p = s.pending.as_mut().ok_or(BridgeError::NotFound)?;
                if !p.actions[index].issued {
                    return Err(BridgeError::IntentRequired);
                }
                p.actions[index].receipt = Some(receipt);
                Ok(())
            },
        )
    }
}
