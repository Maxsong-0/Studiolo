use super::*;

#[test]
fn exact_file_byte_limit_is_allowed_and_one_more_is_rejected() {
    let mut data = input(None, file(1, 1, 11), Presence::Missing);
    data.limits.max_file_bytes = 11;
    assert_eq!(ready(data.clone()).stats().copy_bytes, 11);
    data.limits.max_file_bytes = 10;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
}

#[test]
fn byte_limit_applies_to_the_retained_baseline_too() {
    let mut data = input(Some(common(file(1, 1, 20))), present(2), present(2));
    data.limits.max_file_bytes = 10;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
}

#[test]
fn whole_union_and_total_outputs_are_bounded_without_partial_success() {
    let mut data = baseline_paths(3);
    data.limits.max_paths = 2;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
    let mut data = baseline_paths(3);
    data.limits.max_outputs = 2;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
}

#[test]
fn hard_count_ceiling_cannot_be_raised_by_caller_limits() {
    let mut data = input(None, present(1), Presence::Missing);
    data.limits.max_paths = 4097;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
    let mut data = input(None, present(1), Presence::Missing);
    data.limits.max_outputs = 0;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
    let mut data = baseline_paths(1);
    data.limits.deletion.max_basis_points = 10001;
    assert_eq!(plan(data, None), Err(PlanError::Budget));
}

#[test]
fn giant_logical_path_is_rejected_before_generating_output() {
    let long = VaultPath::parse(format!("项目/{}", "字".repeat(400))).unwrap();
    let mut data = input(None, present(1), Presence::Missing);
    data.scope.paths = BTreeSet::from([long.clone()]);
    data.canonical.entries = BTreeMap::from([(long.clone(), present(1))]);
    data.mirror.entries = BTreeMap::from([(long, Presence::Missing)]);
    assert_eq!(plan(data, None), Err(PlanError::Budget));
}

#[test]
fn aggregate_copy_bytes_use_checked_arithmetic() {
    let mut data = input(None, file(1, 1, u64::MAX), Presence::Missing);
    data.limits.max_file_bytes = u64::MAX;
    data.scope.paths.insert(path(1));
    data.canonical.entries.insert(path(1), file(2, 2, 1));
    data.mirror.entries.insert(path(1), Presence::Missing);
    assert_eq!(plan(data, None), Err(PlanError::Overflow));
}

#[test]
fn a_non_markdown_path_is_processed_only_because_the_exact_scope_contains_it() {
    let data = input(None, present(1), Presence::Missing);
    let result = ready(data);
    assert_eq!(result.actions()[0].key.path, path(0));
    assert!(path(0).as_str().ends_with(".bin"));
}
