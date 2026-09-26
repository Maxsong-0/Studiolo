//! 查询持久原键；Retired 仅拒绝，不能表示历史操作成功。
use super::{BridgeError, BridgeState, model::*, receipts};
use crate::ActionKey;

impl BridgeState {
    /// 旧 floor 只拒绝，不代表历史操作成功。pending/last 保留精确原结果。
    pub fn lookup(&self, key: &ActionKey) -> Result<ProgressView> {
        let s = self.read()?;
        binding_key(s, key)?;
        let p = select(s, key)?;
        let i = receipts::index(p, key)?;
        let action = receipts::plan(p)?.actions()[i].clone();
        view(p, i, action)
    }
    /// 已 Issued 原键的执行数据（冻结动作、原幂等键、请求摘要），供重开后按原键核对写回执。
    /// 只读：不改任何状态，也不是新的执行许可；尚未 Issued 返回 `IntentRequired`。
    pub fn issued_request(&self, key: &ActionKey) -> Result<PendingAction> {
        let s = self.read()?;
        binding_key(s, key)?;
        let p = select(s, key)?;
        let i = receipts::index(p, key)?;
        let progress = p.actions.get(i).ok_or(BridgeError::NotFound)?;
        if !progress.issued {
            return Err(BridgeError::IntentRequired);
        }
        Ok(PendingAction {
            action: receipts::plan(p)?.actions()[i].clone(),
            external_key: progress.key.clone(),
            request_digest: receipts::request_hash(p, i)?,
        })
    }
}

pub(super) fn binding_key(s: &State, key: &ActionKey) -> Result<()> {
    if key.plan.link_id != s.binding.link_id || key.plan.scope_epoch != s.binding.scope_epoch {
        return Err(BridgeError::BindingMismatch);
    }
    if key.plan.sequence == 0 || key.plan.sequence > i64::MAX as u64 {
        return Err(BridgeError::Overflow);
    }
    if key.plan.sequence <= s.floor {
        return Err(BridgeError::Retired);
    }
    Ok(())
}
pub(super) fn select<'a>(s: &'a State, key: &ActionKey) -> Result<&'a Pending> {
    s.pending
        .iter()
        .chain(s.last.iter())
        .find(|p| p.input.plan_sequence == key.plan.sequence)
        .ok_or(BridgeError::NotFound)
}
pub(super) fn view(p: &Pending, i: usize, action: crate::Action) -> Result<ProgressView> {
    let a = p.actions.get(i).ok_or(BridgeError::NotFound)?;
    let status = if let Some(receipt) = &a.receipt {
        ActionStatus::Known(Box::new(receipt.clone()))
    } else if a.issued {
        ActionStatus::OutcomeUnknown
    } else if a.intent_ack.is_some() {
        ActionStatus::Reserved
    } else {
        ActionStatus::IntentPending
    };
    Ok(ProgressView {
        action,
        external_key: a.key.clone(),
        status,
    })
}
