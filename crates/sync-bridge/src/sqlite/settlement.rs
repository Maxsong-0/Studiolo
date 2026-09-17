//! 结果闭合与共同基线分开；已知失败或半成功只记终态，不推进基线。

use super::{BridgeError, BridgeState, audit, model::*, receipts};
use crate::{Baseline, Pair, Presence};
use std::collections::BTreeMap;

impl BridgeState {
    /// 闭合全部结果和审计；仅完整共同新观察可推进基线，None 保持旧基线。
    pub fn settle(
        &mut self,
        expected_version: u64,
        sequence: u64,
        common: Option<CommonObservation>,
    ) -> Result<()> {
        if common.as_ref().is_some_and(|c| {
            c.process_epoch != self.epoch || std::time::Instant::now() >= c.expires_at
        }) {
            return Err(BridgeError::StaleObservation);
        }
        let deadline = common.as_ref().map(|c| c.expires_at);
        self.edit_guarded(
            Edit::Full,
            Some(expected_version),
            || {
                if deadline.is_none_or(|d| std::time::Instant::now() < d) {
                    Ok(())
                } else {
                    Err(BridgeError::StaleObservation)
                }
            },
            |s| {
                if s.fuse.is_some() {
                    return Err(BridgeError::Fused);
                }
                let p = s.pending.as_ref().ok_or(BridgeError::NotFound)?;
                if p.input.plan_sequence != sequence {
                    return Err(BridgeError::RetryMismatch);
                }
                if !receipts::acknowledged(p) {
                    return Err(BridgeError::NotSettled);
                }
                let new_base = common.as_ref().map(|c| common_base(s, p, c)).transpose()?;
                if let Some(last) = &s.last {
                    let retirement = s
                        .retirement
                        .as_ref()
                        .ok_or(BridgeError::RetentionRequired)?;
                    if retirement.ack.is_none()
                        || retirement.old_sequence != last.input.plan_sequence
                        || retirement.old_floor != s.floor
                        || retirement.old_digest != audit::terminal_digest(last)?
                    {
                        return Err(BridgeError::RetentionRequired);
                    }
                    s.floor = retirement.old_sequence;
                }
                if let Some(base) = new_base {
                    s.cursor.baseline_version = base.version;
                    s.cursor.last_canonical_observation = base.canonical_observation;
                    s.cursor.last_mirror_observation = base.mirror_observation;
                    s.baseline = base;
                }
                s.last = s.pending.take();
                s.retirement = None;
                Ok(())
            },
        )
    }
}

fn common_base(s: &State, p: &Pending, c: &CommonObservation) -> Result<Baseline> {
    if !receipts::succeeded(p) || !receipts::plan(p)?.conflicts().is_empty() {
        return Err(BridgeError::NotSettled);
    }
    if c.approved.binding != s.binding
        || c.canonical.binding != s.binding
        || c.mirror.binding != s.binding
    {
        return Err(BridgeError::BindingMismatch);
    }
    if c.canonical.sequence <= s.cursor.last_canonical_observation
        || c.mirror.sequence <= s.cursor.last_mirror_observation
        || c.canonical.sequence > i64::MAX as u64
        || c.mirror.sequence > i64::MAX as u64
        || !p.input.scope.paths.is_subset(&c.approved.paths)
        || c.approved.paths.len() > s.limits.max_paths
    {
        return Err(BridgeError::StaleObservation);
    }
    // 重用纯核对完整性/显式 Missing/所有路径/字节数进行独立检查，输出只能是全量共同观察。
    let input = crate::Input {
        scope: c.approved.clone(),
        baseline: s.baseline.clone(),
        canonical: c.canonical.clone(),
        mirror: c.mirror.clone(),
        cursor: s.cursor.clone(),
        expected_baseline_version: s.baseline.version,
        plan_sequence: s
            .cursor
            .last_plan_sequence
            .checked_add(1)
            .ok_or(BridgeError::Overflow)?,
        limits: s.limits,
    };
    super::input_codec::encode(&input)?;
    let crate::Decision::Ready(plan) =
        crate::plan(input, None).map_err(|_| BridgeError::StaleObservation)?
    else {
        return Err(BridgeError::StaleObservation);
    };
    if !plan.actions().is_empty()
        || !plan.conflicts().is_empty()
        || plan.agreements().len() != c.approved.paths.len()
    {
        return Err(BridgeError::StaleObservation);
    }
    // 新共同内容可以是之后的独立编辑；不把它改写为旧动作的执行回执。
    let mut entries = BTreeMap::new();
    for path in &c.approved.paths {
        let a = c
            .canonical
            .entries
            .get(path)
            .ok_or(BridgeError::StaleObservation)?;
        let b = c
            .mirror
            .entries
            .get(path)
            .ok_or(BridgeError::StaleObservation)?;
        if matches!(
            (a, b),
            (Presence::Missing, Presence::Present(_)) | (Presence::Present(_), Presence::Missing)
        ) {
            return Err(BridgeError::StaleObservation);
        }
        entries.insert(
            path.clone(),
            Pair {
                canonical: a.clone(),
                mirror: b.clone(),
            },
        );
    }
    Ok(Baseline {
        binding: s.binding.clone(),
        version: s
            .baseline
            .version
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(BridgeError::Overflow)?,
        canonical_observation: c.canonical.sequence,
        mirror_observation: c.mirror.sequence,
        entries,
    })
}
