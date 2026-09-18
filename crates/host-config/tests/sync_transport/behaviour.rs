//! 行为契约：显式启用的审批产出封闭能力，并如实报告被批准的值。

use super::fixture::Fixture;

#[test]
fn explicit_enabled_approval_yields_a_closed_capability_with_pinned_roots_and_digest() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    assert_eq!(approved.binding(), scope.binding());
    assert_eq!(
        approved.roots().private_root(),
        fixture.root.join("transport")
    );
    assert_eq!(approved.roots().mirror_root(), fixture.root.join("mirror"));
    assert_ne!(
        approved.roots().private_identity(),
        approved.roots().mirror_identity()
    );
    assert_ne!(approved.approval_digest(), [0u8; 32]);
    assert_eq!(approved.revalidate(), Ok(()));
}

#[test]
fn approved_transport_reports_runtime_backend_and_budget_exactly_as_approved() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();

    assert_eq!(approved.runtime().version(), "0.0.14");
    assert_eq!(
        approved.runtime().node_path(),
        std::path::Path::new("/opt/studiolo/node/bin/node")
    );
    assert_eq!(approved.runtime().closure_digest(), [0x11u8; 32]);
    assert_eq!(
        approved.backend().service_unit(),
        "studiolo-sync-launcher@link-1.service"
    );
    assert_eq!(approved.backend().payload_uid(), 61001);
    assert_eq!(approved.budget().config_json_bytes, 65536);
    assert_eq!(approved.budget().deadline_seconds, 3600);
}

#[test]
fn approval_digest_changes_when_any_approved_value_changes() {
    let fixture = Fixture::new();
    let first = fixture.parse(&fixture.approval()).unwrap().digest();
    let mut other = fixture.approval();
    other["budget"]["pids_max"] = serde_json::json!(65);
    let second = fixture.parse(&other).unwrap().digest();

    assert_ne!(first, second);
}
