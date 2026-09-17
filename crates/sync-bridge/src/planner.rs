//! 完整计算后统一熔断；输出条件动作，不执行或确认任何同步结果。

use super::{
    model::*,
    validation::{fuse, same_content, validate},
};

/// 验证完整证据并计算全批提案；任何旧 pending 均阻止新规划。
pub fn plan(input: Input, pending: Option<&Plan>) -> Result<Decision, PlanError> {
    validate(&input)?;
    if pending.is_some() {
        // 即使当前双方已经相同，也不能把旧未决计划升级成成功。
        return Err(PlanError::PendingUnresolved);
    }
    let key = PlanKey {
        link_id: input.scope.binding.link_id,
        scope_epoch: input.scope.binding.scope_epoch,
        sequence: input.plan_sequence,
    };
    let mut actions = Vec::new();
    let mut agreements = Vec::new();
    let mut conflicts = Vec::new();
    let mut stats = DeleteStats::default();
    for path in &input.scope.paths {
        let current = Pair {
            canonical: input.canonical.entries[path].clone(),
            mirror: input.mirror.entries[path].clone(),
        };
        let base = input.baseline.entries.get(path);
        if base.is_some_and(|base| matches!(base.canonical, Presence::Present(_))) {
            stats.baseline_live_paths += 1;
        }
        match decide(base, &current) {
            Item::Agreement => agreements.push(AgreementCandidate {
                path: path.clone(),
                observed: current,
                expected_baseline_version: input.expected_baseline_version,
            }),
            Item::Conflict(reason) => conflicts.push(Conflict {
                path: path.clone(),
                reason,
                baseline: base.cloned(),
                observed: current,
            }),
            Item::Change(target, change) => {
                let operation = match &change {
                    Change::CreateOnly { source } | Change::Replace { source, .. } => {
                        stats.copy_bytes = stats
                            .copy_bytes
                            .checked_add(source.bytes)
                            .ok_or(PlanError::Overflow)?;
                        if matches!(change, Change::CreateOnly { .. }) {
                            OperationKind::CreateOnly
                        } else {
                            OperationKind::Replace
                        }
                    }
                    Change::Delete { .. } => {
                        match target {
                            Side::Canonical => stats.canonical += 1,
                            Side::Mirror => stats.mirror += 1,
                        }
                        OperationKind::Delete
                    }
                };
                actions.push(Action {
                    key: ActionKey {
                        plan: key.clone(),
                        path: path.clone(),
                        target,
                        operation,
                    },
                    change,
                });
            }
        }
    }
    // 每路径最多一项输出，输入全路径验证完成前不会返回部分动作。
    if actions.len() + agreements.len() + conflicts.len() > input.limits.max_outputs {
        return Err(PlanError::Budget);
    }
    if let Some(reason) = fuse(stats, input.limits.deletion)? {
        return Ok(Decision::FuseBlocked(FuseBlocked {
            key,
            stats,
            reason,
            conflicts,
        }));
    }
    Ok(Decision::Ready(Box::new(Plan {
        original: input,
        key,
        actions,
        agreements,
        conflicts,
        stats,
    })))
}

/// 只重发同一冻结提案；不会核对磁盘或伪造提交回执。
/// 传输/持久层仍须保证原 pending 身份及执行前条件，不能用新观察替换原输入。
pub fn retry(original: &Plan, frozen: &Input) -> Result<Plan, PlanError> {
    if &original.original != frozen {
        return Err(PlanError::RetryMismatch);
    }
    validate(frozen)?;
    Ok(original.clone())
}

enum Item {
    Agreement,
    Conflict(ConflictReason),
    Change(Side, Change),
}

fn decide(base: Option<&Pair>, current: &Pair) -> Item {
    if same_content(&current.canonical, &current.mirror) {
        return Item::Agreement;
    }
    let Some(base) = base else {
        return match (&current.canonical, &current.mirror) {
            (Presence::Missing, Presence::Present(file)) => Item::Change(
                Side::Canonical,
                Change::CreateOnly {
                    source: file.clone(),
                },
            ),
            (Presence::Present(file), Presence::Missing) => Item::Change(
                Side::Mirror,
                Change::CreateOnly {
                    source: file.clone(),
                },
            ),
            _ => Item::Conflict(ConflictReason::FirstDivergence),
        };
    };
    let canonical_changed = current.canonical != base.canonical;
    let mirror_changed = current.mirror != base.mirror;
    match (canonical_changed, mirror_changed) {
        (true, false) => copy(&current.canonical, &current.mirror, Side::Mirror),
        (false, true) => copy(&current.mirror, &current.canonical, Side::Canonical),
        (true, true)
            if matches!(current.canonical, Presence::Missing)
                || matches!(current.mirror, Presence::Missing) =>
        {
            Item::Conflict(ConflictReason::DeleteAgainstModification)
        }
        _ => Item::Conflict(ConflictReason::BothChanged),
    }
}

fn copy(source: &Presence, target: &Presence, side: Side) -> Item {
    match (source, target) {
        (Presence::Present(source), Presence::Missing) => Item::Change(
            side,
            Change::CreateOnly {
                source: source.clone(),
            },
        ),
        (Presence::Present(source), Presence::Present(target)) => Item::Change(
            side,
            Change::Replace {
                source: source.clone(),
                target_expected: target.clone(),
            },
        ),
        (Presence::Missing, Presence::Present(target)) => Item::Change(
            side,
            Change::Delete {
                target_expected: target.clone(),
            },
        ),
        (Presence::Missing, Presence::Missing) => Item::Agreement,
    }
}
