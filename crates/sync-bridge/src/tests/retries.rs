use super::*;

#[test]
fn exact_retry_preserves_the_frozen_inputs_plan_key_and_action_keys() {
    let data = input(None, Presence::Missing, present(1));
    let original = ready(data.clone());
    let retried = retry(&original, &data).unwrap();
    assert_eq!(retried, original);
    assert_eq!(retried.actions()[0].key, original.actions()[0].key);
    assert_eq!(
        plan(data, Some(&original)),
        Err(PlanError::PendingUnresolved)
    );
}

#[test]
fn retry_cannot_substitute_new_evidence_scope_cursor_or_limits() {
    let original_input = input(None, Presence::Missing, present(1));
    let original = ready(original_input.clone());
    for change in 0..7 {
        let mut changed = original_input.clone();
        match change {
            0 => {
                changed.mirror.entries.insert(path(0), present(2));
            }
            1 => changed.scope.binding.scope_hash[0] ^= 1,
            2 => changed.canonical.sequence += 1,
            3 => changed.plan_sequence += 1,
            4 => changed.expected_baseline_version += 1,
            5 => changed.limits.max_file_bytes += 1,
            _ => changed.cursor.last_plan_sequence += 1,
        };
        assert_eq!(retry(&original, &changed), Err(PlanError::RetryMismatch));
    }
}

#[test]
fn coincidental_equality_cannot_confirm_an_unknown_previous_plan() {
    let original = ready(input(None, Presence::Missing, present(1)));
    let mut next = input(None, present(1), present(1));
    next.cursor.last_plan_sequence = 1;
    next.cursor.last_canonical_observation = 1;
    next.cursor.last_mirror_observation = 1;
    next.canonical.sequence = 2;
    next.mirror.sequence = 2;
    next.plan_sequence = 2;
    assert_eq!(
        plan(next.clone(), Some(&original)),
        Err(PlanError::PendingUnresolved)
    );
    // 没有 pending 的独立新观察只能建议共同基线，没有提交回执。
    let observation = ready(next);
    assert!(observation.actions().is_empty());
    assert_eq!(observation.agreements().len(), 1);
    assert!(original.frozen_input().baseline.entries.is_empty());
}

#[test]
fn a_to_b_to_a_uses_distinct_monotonic_plan_keys() {
    let a = ready(input(None, Presence::Missing, present(1)));
    // 这里显式构造已由持久层确认的新基线，不由 planner 自动推进。
    let b = ready(input(Some(common(present(1))), present(1), present(2)));
    let mut again = input(Some(common(present(2))), present(2), present(1));
    again.baseline.version = 2;
    again.baseline.canonical_observation = 2;
    again.baseline.mirror_observation = 2;
    again.expected_baseline_version = 2;
    again.cursor.baseline_version = 2;
    again.cursor.last_plan_sequence = 2;
    again.cursor.last_canonical_observation = 2;
    again.cursor.last_mirror_observation = 2;
    again.plan_sequence = 3;
    again.canonical.sequence = 3;
    again.mirror.sequence = 3;
    let again = ready(again);
    assert_eq!(
        (a.key().sequence, b.key().sequence, again.key().sequence),
        (1, 2, 3)
    );
    assert_ne!(a.key(), again.key());
    assert_ne!(b.actions()[0].key, again.actions()[0].key);
    assert_eq!(retry(&b, b.frozen_input()).unwrap(), b);
}

#[test]
fn order_is_deterministic_across_map_insertion_order() {
    let mut data = baseline_paths(3);
    data.mirror.entries.insert(path(2), present(2));
    data.mirror.entries.insert(path(0), present(2));
    let mut reversed = data.clone();
    reversed.mirror.entries = data.mirror.entries.clone().into_iter().rev().collect();
    assert_eq!(ready(reversed), ready(data));
}
