use super::*;

#[test]
fn first_observation_only_copies_into_explicitly_missing_target() {
    for (canonical, mirror, target) in [
        (Presence::Missing, present(2), Side::Canonical),
        (present(2), Presence::Missing, Side::Mirror),
    ] {
        let result = ready(input(None, canonical, mirror));
        assert_eq!(result.actions().len(), 1);
        assert_eq!(result.actions()[0].key.target, target);
        assert_eq!(result.actions()[0].key.operation, OperationKind::CreateOnly);
        assert!(matches!(
            result.actions()[0].change,
            Change::CreateOnly { .. }
        ));
        assert!(result.agreements().is_empty());
        assert_eq!(result.stats().canonical + result.stats().mirror, 0);
    }
}

#[test]
fn first_divergence_keeps_both_versions_and_never_selects_a_winner() {
    let result = ready(input(None, present(2), present(3)));
    assert!(result.actions().is_empty());
    let conflict = &result.conflicts()[0];
    assert_eq!(conflict.reason, ConflictReason::FirstDivergence);
    assert_eq!(conflict.baseline, None);
    assert_eq!(
        conflict.observed,
        Pair {
            canonical: present(2),
            mirror: present(3)
        }
    );
}

#[test]
fn equal_bytes_with_distinct_side_revisions_are_only_an_agreement_candidate() {
    let canonical = file(10, 2, 10);
    let mirror = file(20, 2, 10);
    let result = ready(input(
        Some(common(present(1))),
        canonical.clone(),
        mirror.clone(),
    ));
    assert!(result.actions().is_empty());
    assert!(result.conflicts().is_empty());
    assert_eq!(result.agreements()[0].observed, Pair { canonical, mirror });
    assert_eq!(result.agreements()[0].expected_baseline_version, 1);
    assert_eq!(
        result.frozen_input().baseline.entries[&path(0)],
        common(present(1))
    );
}

#[test]
fn first_equal_and_both_deleted_emit_no_mutation_or_history_request() {
    for (base, current) in [
        (None, present(1)),
        (Some(common(present(1))), Presence::Missing),
    ] {
        let result = ready(input(base, current.clone(), current));
        assert!(result.actions().is_empty());
        assert_eq!(result.agreements().len(), 1);
        assert_eq!(result.stats().copy_bytes, 0);
    }
}

#[test]
fn one_changed_side_preserves_each_sides_exact_revision_precondition() {
    let old_canonical = file(11, 1, 10);
    let old_mirror = file(12, 1, 10);
    let baseline = Pair {
        canonical: old_canonical.clone(),
        mirror: old_mirror.clone(),
    };
    for (canonical, mirror, target, expected) in [
        (present(2), old_mirror.clone(), Side::Mirror, old_mirror),
        (
            old_canonical.clone(),
            present(2),
            Side::Canonical,
            old_canonical,
        ),
    ] {
        let result = ready(input(Some(baseline.clone()), canonical, mirror));
        let action = &result.actions()[0];
        assert_eq!(action.key.target, target);
        let Presence::Present(expected) = expected else {
            panic!()
        };
        assert_eq!(
            action.change,
            Change::Replace {
                source: FileVersion {
                    revision: [2; 32],
                    sha256: [2; 32],
                    bytes: 10
                },
                target_expected: expected,
            }
        );
    }
}

#[test]
fn both_changed_differently_keep_base_and_both_current_versions() {
    let result = ready(input(Some(common(present(1))), present(2), present(3)));
    assert!(result.actions().is_empty());
    assert_eq!(
        result.conflicts()[0],
        Conflict {
            path: path(0),
            reason: ConflictReason::BothChanged,
            baseline: Some(common(present(1))),
            observed: Pair {
                canonical: present(2),
                mirror: present(3)
            },
        }
    );
}

#[test]
fn previously_common_missing_file_is_created_without_delete_semantics() {
    let result = ready(input(
        Some(common(Presence::Missing)),
        present(2),
        Presence::Missing,
    ));
    assert!(matches!(
        result.actions()[0].change,
        Change::CreateOnly { .. }
    ));
    assert_eq!(result.stats().baseline_live_paths, 0);
}
