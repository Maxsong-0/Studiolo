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
