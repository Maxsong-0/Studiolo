//! 在真实采集已完成后改变批准，验证最后发布门禁不会留下前段成功图。
mod fixture;
use fixture::Fixture;

#[test]
fn approved_builders_cannot_add_cargo_or_any_rust_source_authority() {
    let f = Fixture::new(false);
    let repository = f.repository().with_cargo_dependencies();
    assert_eq!(repository.observe_cargo_dependencies().status, "disabled");
    assert!(
        repository
            .with_rust_call_sites(vec!["src/lib.rs".into()])
            .is_err()
    );
    let allowed = Fixture::new(true);
    assert_eq!(
        allowed
            .repository()
            .with_cargo_dependencies()
            .observe_cargo_dependencies()
            .status,
        "ready"
    );
    assert!(
        allowed
            .repository()
            .with_rust_call_sites(Vec::new())
            .is_err()
    );
}

#[test]
fn approval_change_after_successful_cargo_capture_erases_the_entire_evidence_group() {
    let f = Fixture::new(true);
    let repository = f.repository();
    let mut cargo = repository.observe_cargo_dependencies();
    let mut rust = repository.observe_rust_call_sites();
    let mut directories = repository.observe(&[String::new()]);
    assert_eq!(cargo.status, "ready");
    assert_eq!(directories.status, "ready");
    assert!(cargo.graph.is_some() && cargo.observed_at.is_some());
    f.change_config();
    super::finalize_approval(&repository, &mut directories, &mut cargo, &mut rust);
    assert_eq!(directories.status, "unavailable");
    assert!(directories.observations.is_empty() && directories.observed_at.is_none());
    assert_eq!(cargo.status, "unavailable");
    assert!(cargo.graph.is_none() && cargo.observed_at.is_none());
    assert_eq!(rust.status, "disabled");
    assert!(rust.graph.is_none() && rust.links.is_none() && rust.observed_at.is_none());
}

#[test]
fn repository_replacement_after_capture_cannot_publish_the_prior_ready_graph() {
    let f = Fixture::new(true);
    let repository = f.repository();
    let mut cargo = repository.observe_cargo_dependencies();
    let mut rust = repository.observe_rust_call_sites();
    let mut directories = repository.observe(&[String::new()]);
    assert_eq!(cargo.status, "ready");
    f.replace_root();
    super::finalize_approval(&repository, &mut directories, &mut cargo, &mut rust);
    assert_eq!(cargo.status, "unavailable");
    assert!(cargo.graph.is_none() && cargo.observed_at.is_none());
    assert_eq!(directories.status, "unavailable");
}
