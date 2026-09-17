//! 打开/完整更新验证全部关系，单动作验证自身；拒绝损坏而不降级空状态。

use super::{BridgeError, MAX_PATHS, audit, input_codec, model::*, receipts};
use crate::{Decision, Presence};

pub(crate) fn state(s: &State) -> Result<()> {
    if s.binding != s.cursor.binding
        || s.binding != s.baseline.binding
        || s.binding.canonical_root_token == s.binding.mirror_root_token
    {
        return Err(BridgeError::BindingMismatch);
    }
    if s.limits.max_paths == 0
        || s.limits.max_paths > MAX_PATHS
        || s.limits.max_outputs == 0
        || s.limits.max_outputs > MAX_PATHS
        || s.limits.deletion.max_count != 5
        || s.limits.deletion.max_basis_points != 1000
        || s.limits.max_file_bytes > i64::MAX as u64
        || s.binding.scope_epoch > i64::MAX as u64
    {
        return Err(BridgeError::Budget);
    }
    if s.version > i64::MAX as u64
        || s.cursor.last_plan_sequence > i64::MAX as u64
        || s.cursor.last_canonical_observation > i64::MAX as u64
        || s.cursor.last_mirror_observation > i64::MAX as u64
    {
        return Err(BridgeError::Overflow);
    }
    baseline(s)?;
    if s.floor > s.cursor.last_plan_sequence || s.pending.is_some() && s.fuse.is_some() {
        return Err(BridgeError::CorruptStore);
    }
    if let Some(p) = &s.pending {
        pending(p, s)?;
        if p.input.baseline != s.baseline
            || p.input.plan_sequence != s.cursor.last_plan_sequence
            || p.input.canonical.sequence != s.cursor.last_canonical_observation
            || p.input.mirror.sequence != s.cursor.last_mirror_observation
        {
            return Err(BridgeError::CorruptStore);
        }
    }
    if let Some(p) = &s.last {
        pending(p, s)?;
        if !receipts::acknowledged(p)
            || p.input.plan_sequence <= s.floor
            || p.input.plan_sequence > s.cursor.last_plan_sequence
            || s.pending
                .as_ref()
                .is_some_and(|q| p.input.plan_sequence >= q.input.plan_sequence)
        {
            return Err(BridgeError::CorruptStore);
        }
    } else if s.floor != 0 || s.retirement.is_some() {
        return Err(BridgeError::CorruptStore);
    }
    let latest = s
        .pending
        .as_ref()
        .or(s.last.as_ref())
        .map_or(0, |p| p.input.plan_sequence);
    if let Some(f) = &s.fuse {
        input_codec::encode(&f.input)?;
        if f.input.scope.binding != s.binding
            || f.input.baseline != s.baseline
            || f.input.limits != s.limits
            || f.input.plan_sequence != s.cursor.last_plan_sequence
            || f.input.plan_sequence <= latest
            || f.input.canonical.sequence != s.cursor.last_canonical_observation
            || f.input.mirror.sequence != s.cursor.last_mirror_observation
            || !matches!(
                crate::plan(f.input.clone(), None),
                Ok(Decision::FuseBlocked(_))
            )
        {
            return Err(BridgeError::CorruptStore);
        }
        if let Some(ack) = &f.ack {
            audit::check_ack(ack, &audit::event(s, audit::Slot::Fuse)?)?;
        }
    } else if latest != s.cursor.last_plan_sequence {
        return Err(BridgeError::CorruptStore);
    }
    if let Some(r) = &s.retirement {
        let last = s.last.as_ref().ok_or(BridgeError::CorruptStore)?;
        if r.old_sequence != last.input.plan_sequence
            || r.old_floor != s.floor
            || r.old_digest != audit::terminal_digest(last)?
        {
            return Err(BridgeError::CorruptStore);
        }
        if let Some(ack) = &r.ack {
            audit::check_ack(ack, &audit::event(s, audit::Slot::Retirement)?)?;
        }
    }
    super::capacity::validate(s)
}
fn pending(p: &Pending, s: &State) -> Result<()> {
    input_codec::encode(&p.input)?;
    if p.input.scope.binding != s.binding
        || p.input.limits != s.limits
        || p.actions.len() > MAX_PATHS
    {
        return Err(BridgeError::CorruptStore);
    }
    let plan = receipts::plan(p)?;
    if p.actions.len() != plan.actions().len() {
        return Err(BridgeError::CorruptStore);
    }
    if p.cached_hash != receipts::input_hash(&p.input)? {
        return Err(BridgeError::CorruptStore);
    }
    for i in 0..p.actions.len() {
        progress(p, i)?;
    }
    if p.summary_ack.is_some()
        && (!receipts::known(p) || p.actions.iter().any(|a| a.outcome_ack.is_none()))
    {
        return Err(BridgeError::CorruptStore);
    }
    audit::check_summary(p)
}
pub(crate) fn progress(p: &Pending, i: usize) -> Result<()> {
    let a = &p.actions[i];
    if a.key != receipts::external_from_hash(&p.input, p.cached_hash, i)
        || a.issued && a.intent_ack.is_none()
        || a.receipt.is_some() && !a.issued
        || a.outcome_ack.is_some() && a.receipt.is_none()
    {
        return Err(BridgeError::CorruptStore);
    }
    if let Some(receipt) = &a.receipt {
        receipts::validate(p, i, receipt)?;
    }
    audit::check_action(p, i)
}
fn baseline(s: &State) -> Result<()> {
    let b = &s.baseline;
    if b.version != s.cursor.baseline_version
        || b.entries.len() > s.limits.max_paths
        || b.canonical_observation > s.cursor.last_canonical_observation
        || b.mirror_observation > s.cursor.last_mirror_observation
        || b.version == 0
            && (!b.entries.is_empty() || b.canonical_observation != 0 || b.mirror_observation != 0)
        || b.version > 0 && (b.canonical_observation == 0 || b.mirror_observation == 0)
    {
        return Err(BridgeError::CorruptStore);
    }
    for (path, pair) in &b.entries {
        if path.as_str().len() > 1024 {
            return Err(BridgeError::Budget);
        }
        match (&pair.canonical, &pair.mirror) {
            (Presence::Missing, Presence::Missing) => {}
            (Presence::Present(a), Presence::Present(b))
                if a.sha256 == b.sha256
                    && a.bytes == b.bytes
                    && a.bytes <= s.limits.max_file_bytes => {}
            _ => return Err(BridgeError::CorruptStore),
        }
    }
    Ok(())
}
