//! 验证全批证据和显式缺失；没有按页或按前缀跳过验证的分支。

use super::model::*;
use std::collections::BTreeMap;

pub(super) fn same_content(left: &Presence, right: &Presence) -> bool {
    match (left, right) {
        (Presence::Missing, Presence::Missing) => true,
        (Presence::Present(left), Presence::Present(right)) => {
            left.sha256 == right.sha256 && left.bytes == right.bytes
        }
        _ => false,
    }
}

pub(super) fn validate(input: &Input) -> Result<(), PlanError> {
    let limits = input.limits;
    if limits.max_paths == 0
        || limits.max_paths > 4096
        || limits.max_outputs == 0
        || limits.max_outputs > 4096
        || limits.deletion.max_count > 4096
        || limits.deletion.max_basis_points > 10000
        || input.scope.paths.len() > limits.max_paths
        || input.scope.paths.len() > limits.max_outputs
        || input.baseline.entries.len() > limits.max_paths
        || input.canonical.entries.len() > limits.max_paths
        || input.mirror.entries.len() > limits.max_paths
    {
        return Err(PlanError::Budget);
    }
    let expected = &input.scope.binding;
    if expected.canonical_root_token == expected.mirror_root_token
        || [
            &input.baseline.binding,
            &input.canonical.binding,
            &input.mirror.binding,
            &input.cursor.binding,
        ]
        .into_iter()
        .any(|binding| binding != expected)
    {
        return Err(PlanError::BindingMismatch);
    }
    if input.canonical.side != Side::Canonical || input.mirror.side != Side::Mirror {
        return Err(PlanError::WrongSide);
    }
    if !input.canonical.complete || !input.mirror.complete {
        return Err(PlanError::IncompleteObservation);
    }
    if input.baseline.version != input.expected_baseline_version
        || input.cursor.baseline_version != input.expected_baseline_version
    {
        return Err(PlanError::BaselineVersionMismatch);
    }
    if input.cursor.last_plan_sequence.checked_add(1) != Some(input.plan_sequence)
        || input.canonical.sequence <= input.cursor.last_canonical_observation
        || input.mirror.sequence <= input.cursor.last_mirror_observation
        || input.baseline.canonical_observation > input.cursor.last_canonical_observation
        || input.baseline.mirror_observation > input.cursor.last_mirror_observation
    {
        return Err(PlanError::SequenceMismatch);
    }
    if (input.baseline.version == 0
        && (!input.baseline.entries.is_empty()
            || input.baseline.canonical_observation != 0
            || input.baseline.mirror_observation != 0))
        || (input.baseline.version != 0
            && (input.baseline.canonical_observation == 0
                || input.baseline.mirror_observation == 0))
    {
        return Err(PlanError::InvalidBaseline);
    }
    for inventory in [&input.canonical, &input.mirror] {
        if !inventory.entries.keys().eq(input.scope.paths.iter()) {
            return Err(PlanError::ScopeMismatch);
        }
    }
    if input
        .baseline
        .entries
        .keys()
        .any(|path| !input.scope.paths.contains(path))
    {
        return Err(PlanError::ScopeMismatch);
    }
    // VaultPath 负责语法/NFC；纯规划元数据仍设独立长度预算，避免巨型路径。
    if input
        .scope
        .paths
        .iter()
        .any(|path| path.as_str().len() > 1024)
    {
        return Err(PlanError::Budget);
    }
    let mut hashes = BTreeMap::new();
    for path in &input.scope.paths {
        let canonical = &input.canonical.entries[path];
        let mirror = &input.mirror.entries[path];
        check_file(canonical, limits.max_file_bytes, &mut hashes)?;
        check_file(mirror, limits.max_file_bytes, &mut hashes)?;
        if let Some(base) = input.baseline.entries.get(path) {
            check_file(&base.canonical, limits.max_file_bytes, &mut hashes)?;
            check_file(&base.mirror, limits.max_file_bytes, &mut hashes)?;
            if !same_content(&base.canonical, &base.mirror) {
                return Err(PlanError::InvalidBaseline);
            }
            same_revision(&base.canonical, canonical)?;
            same_revision(&base.mirror, mirror)?;
        }
    }
    Ok(())
}

fn check_file(
    value: &Presence,
    max: u64,
    hashes: &mut BTreeMap<Digest, u64>,
) -> Result<(), PlanError> {
    if let Presence::Present(file) = value {
        if file.bytes > max {
            return Err(PlanError::Budget);
        }
        if hashes
            .insert(file.sha256, file.bytes)
            .is_some_and(|bytes| bytes != file.bytes)
        {
            return Err(PlanError::EvidenceContradiction);
        }
    }
    Ok(())
}

fn same_revision(base: &Presence, current: &Presence) -> Result<(), PlanError> {
    if let (Presence::Present(base), Presence::Present(current)) = (base, current)
        && base.revision == current.revision
        && base != current
    {
        return Err(PlanError::EvidenceContradiction);
    }
    Ok(())
}

pub(super) fn fuse(
    stats: DeleteStats,
    limits: DeleteLimits,
) -> Result<Option<FuseReason>, PlanError> {
    let count = stats
        .canonical
        .checked_add(stats.mirror)
        .ok_or(PlanError::Overflow)?;
    let over_count = count > limits.max_count;
    let numerator = count.checked_mul(10000).ok_or(PlanError::Overflow)?;
    let denominator = stats
        .baseline_live_paths
        .checked_mul(u64::from(limits.max_basis_points))
        .ok_or(PlanError::Overflow)?;
    let over_ratio = numerator > denominator;
    Ok(match (over_count, over_ratio) {
        (false, false) => None,
        (true, false) => Some(FuseReason::Count),
        (false, true) => Some(FuseReason::Ratio),
        (true, true) => Some(FuseReason::CountAndRatio),
    })
}
