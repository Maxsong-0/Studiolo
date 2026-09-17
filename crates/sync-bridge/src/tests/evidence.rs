use super::*;

#[test]
fn either_incomplete_inventory_blocks_creates_updates_and_deletes() {
    for baseline in [None, Some(common(present(1)))] {
        for side in [Side::Canonical, Side::Mirror] {
            let mut data = input(baseline.clone(), present(2), Presence::Missing);
            match side {
                Side::Canonical => data.canonical.complete = false,
                Side::Mirror => data.mirror.complete = false,
            }
            assert_eq!(plan(data, None), Err(PlanError::IncompleteObservation));
        }
    }
}

#[test]
fn omitted_entries_are_not_missing_and_new_paths_need_both_sides() {
    let mut data = input(None, present(1), Presence::Missing);
    data.mirror.entries.remove(&path(0));
    assert_eq!(plan(data, None), Err(PlanError::ScopeMismatch));
    let mut data = baseline_paths(1);
    data.scope.paths.insert(path(1));
    data.mirror.entries.insert(path(1), present(2));
    assert_eq!(plan(data, None), Err(PlanError::ScopeMismatch));
}

#[test]
fn unapproved_paths_and_baseline_paths_outside_scope_are_rejected() {
    let mut data = input(None, present(1), Presence::Missing);
    data.canonical.entries.insert(path(1), present(2));
    assert_eq!(plan(data, None), Err(PlanError::ScopeMismatch));
    let mut data = baseline_paths(1);
    data.baseline.entries.insert(path(1), common(present(2)));
    assert_eq!(plan(data, None), Err(PlanError::ScopeMismatch));
}

#[test]
fn every_binding_dimension_is_checked_on_every_evidence_container() {
    for slot in 0..4 {
        for dimension in 0..7 {
            let mut data = baseline_paths(1);
            let value = match slot {
                0 => &mut data.baseline.binding,
                1 => &mut data.canonical.binding,
                2 => &mut data.mirror.binding,
                _ => &mut data.cursor.binding,
            };
            match dimension {
                0 => value.link_id[0] ^= 1,
                1 => value.canonical_vault[0] ^= 1,
                2 => value.remote_vault[0] ^= 1,
                3 => value.canonical_root_token[0] ^= 1,
                4 => value.mirror_root_token[0] ^= 1,
                5 => value.scope_hash[0] ^= 1,
                _ => value.scope_epoch += 1,
            }
            assert_eq!(plan(data, None), Err(PlanError::BindingMismatch));
        }
    }
}

#[test]
fn reversed_sides_and_same_physical_root_are_rejected() {
    let mut data = baseline_paths(1);
    data.canonical.side = Side::Mirror;
    assert_eq!(plan(data, None), Err(PlanError::WrongSide));
    let mut data = baseline_paths(1);
    data.scope.binding.mirror_root_token = data.scope.binding.canonical_root_token;
    assert_eq!(plan(data, None), Err(PlanError::BindingMismatch));
}

#[test]
fn common_baseline_cannot_be_divergent_or_half_missing() {
    for base in [
        Pair {
            canonical: present(1),
            mirror: present(2),
        },
        Pair {
            canonical: present(1),
            mirror: Presence::Missing,
        },
    ] {
        let data = input(Some(base), present(2), present(2));
        assert_eq!(plan(data, None), Err(PlanError::InvalidBaseline));
    }
}

#[test]
fn same_revision_with_different_bytes_and_same_hash_with_different_size_are_invalid() {
    let data = input(Some(common(present(1))), file(1, 2, 10), present(1));
    assert_eq!(plan(data, None), Err(PlanError::EvidenceContradiction));
    let data = input(None, file(1, 2, 10), file(2, 2, 11));
    assert_eq!(plan(data, None), Err(PlanError::EvidenceContradiction));
}

#[test]
fn baseline_version_is_bound_to_both_explicit_expectation_and_cursor() {
    for change in 0..3 {
        let mut data = baseline_paths(1);
        match change {
            0 => data.expected_baseline_version += 1,
            1 => data.cursor.baseline_version += 1,
            _ => data.baseline.version += 1,
        }
        assert_eq!(plan(data, None), Err(PlanError::BaselineVersionMismatch));
    }
}

#[test]
fn observations_and_plan_sequences_never_regress_or_reuse() {
    for change in 0..6 {
        let mut data = baseline_paths(1);
        match change {
            0 => data.canonical.sequence = 1,
            1 => data.mirror.sequence = 0,
            2 => data.plan_sequence = 1,
            3 => data.plan_sequence = 3,
            4 => data.baseline.canonical_observation = 2,
            _ => {
                data.cursor.last_plan_sequence = u64::MAX;
                data.plan_sequence = 0;
            }
        }
        assert_eq!(plan(data, None), Err(PlanError::SequenceMismatch));
    }
}
