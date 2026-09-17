use super::*;

#[test]
fn deletion_against_modification_preserves_both_evidence_slots() {
    for (canonical, mirror) in [
        (Presence::Missing, present(2)),
        (present(2), Presence::Missing),
    ] {
        let result = ready(input(
            Some(common(present(1))),
            canonical.clone(),
            mirror.clone(),
        ));
        assert!(result.actions().is_empty());
        assert_eq!(
            result.conflicts()[0].reason,
            ConflictReason::DeleteAgainstModification
        );
        assert_eq!(result.conflicts()[0].observed, Pair { canonical, mirror });
        assert_eq!(result.stats().canonical + result.stats().mirror, 0);
    }
}

#[test]
fn exact_ten_percent_and_count_five_are_allowed_with_target_evidence() {
    let mut data = baseline_paths(50);
    for number in 0..5 {
        data.mirror.entries.insert(path(number), Presence::Missing);
    }
    let result = ready(data);
    assert_eq!(result.actions().len(), 5);
    assert_eq!(result.stats().canonical, 5);
    assert_eq!(result.stats().baseline_live_paths, 50);
    assert!(
        result
            .actions()
            .iter()
            .all(|action| action.key.target == Side::Canonical
                && matches!(action.change, Change::Delete { .. }))
    );
    let Change::Delete { target_expected } = &result.actions()[0].change else {
        panic!()
    };
    assert_eq!(target_expected.revision, [1; 32]);
}

#[test]
fn one_deletion_in_small_baseline_is_blocked_by_ratio() {
    let mut data = baseline_paths(9);
    data.canonical.entries.insert(path(0), Presence::Missing);
    let Decision::FuseBlocked(report) = plan(data, None).unwrap() else {
        panic!()
    };
    assert_eq!(report.reason, FuseReason::Ratio);
    assert_eq!(report.stats.mirror, 1);
    assert_eq!(report.stats.baseline_live_paths, 9);
}

#[test]
fn all_directions_are_counted_before_any_action_is_exposed() {
    let mut data = baseline_paths(100);
    for number in 0..3 {
        data.canonical
            .entries
            .insert(path(number), Presence::Missing);
    }
    for number in 3..6 {
        data.mirror.entries.insert(path(number), Presence::Missing);
    }
    // 同批还有普通修改；熔断结果没有 actions 字段，不会泄出可执行前半批。
    data.mirror.entries.insert(path(99), present(2));
    let Decision::FuseBlocked(report) = plan(data, None).unwrap() else {
        panic!()
    };
    assert_eq!(report.reason, FuseReason::Count);
    assert_eq!(report.stats.canonical, 3);
    assert_eq!(report.stats.mirror, 3);
    assert_eq!(report.stats.copy_bytes, 10);
}

#[test]
fn new_files_cannot_inflate_the_denominator() {
    let mut data = baseline_paths(1);
    data.mirror.entries.insert(path(0), Presence::Missing);
    for number in 1..100 {
        data.scope.paths.insert(path(number));
        data.canonical
            .entries
            .insert(path(number), Presence::Missing);
        data.mirror.entries.insert(path(number), present(2));
    }
    let Decision::FuseBlocked(report) = plan(data, None).unwrap() else {
        panic!()
    };
    assert_eq!(report.stats.baseline_live_paths, 1);
    assert_eq!(report.reason, FuseReason::Ratio);
}

#[test]
fn presenting_one_page_while_keeping_full_baseline_is_rejected() {
    let mut data = baseline_paths(60);
    for number in 1..60 {
        data.scope.paths.remove(&path(number));
        data.canonical.entries.remove(&path(number));
        data.mirror.entries.remove(&path(number));
    }
    data.mirror.entries.insert(path(0), Presence::Missing);
    assert_eq!(plan(data, None), Err(PlanError::ScopeMismatch));
}

#[test]
fn threshold_cross_multiplication_does_not_wrap() {
    let limits = Limits::conservative(10).deletion;
    for stats in [
        DeleteStats {
            canonical: u64::MAX,
            mirror: 1,
            ..DeleteStats::default()
        },
        DeleteStats {
            canonical: u64::MAX / 10000 + 1,
            ..DeleteStats::default()
        },
        DeleteStats {
            baseline_live_paths: u64::MAX,
            ..DeleteStats::default()
        },
    ] {
        assert_eq!(
            super::super::validation::fuse(stats, limits),
            Err(PlanError::Overflow)
        );
    }
}
