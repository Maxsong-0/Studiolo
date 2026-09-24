//! 行为契约：显式启用的审批产出封闭能力，并如实报告被批准的值。

use super::fixture::Fixture;
use studiolo_host_config::sync_transport::{ManualQuota, TransportSyncMode};

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

/// 能力活着期间一直攥着两个根的 FD：反复复验必须稳定通过，报告的取值也不漂。
#[test]
fn approved_roots_stay_pinned_to_the_same_objects_across_revalidation() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();
    let private = approved.roots().private_identity();
    let mirror = approved.roots().mirror_identity();

    for _ in 0..3 {
        assert_eq!(approved.revalidate(), Ok(()));
        assert_eq!(approved.roots().private_identity(), private);
        assert_eq!(approved.roots().mirror_identity(), mirror);
    }

    // 同一份配置可以并存第二份批准；两份钉的是同一批对象。
    let second = fixture.approve(&scope).unwrap();
    assert_eq!(second.roots().private_identity(), private);
    assert_eq!(second.roots().mirror_identity(), mirror);
    assert_eq!(second.revalidate(), Ok(()));
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

/// 排除项按母本排序、设备名与模式逐字报告；人工配额显式 `null` 即未知，给了值才是 `Some`。
#[test]
fn excluded_folders_mode_device_and_manual_quota_are_reported_exactly_as_approved() {
    let fixture = Fixture::new();
    let mut value = fixture.approval();
    value["excluded_folders"] = serde_json::json!(["日记", "School", "Archive/2025"]);
    let approval = fixture.parse(&value).unwrap();
    assert_eq!(
        approval.excluded_folders(),
        ["Archive/2025", "School", "日记"]
    );
    assert_eq!(approval.device_name(), "studiolo-fixture");
    assert_eq!(approval.headless_mode(), TransportSyncMode::MirrorRemote);
    assert_eq!(approval.manual_quota(), None);

    value["headless_mode"] = serde_json::json!("bidirectional");
    value["manual_quota"] = serde_json::json!({
        "account_limit_bytes": 10_737_418_240_u64,
        "used_bytes_manual": 4_294_967_296_u64,
        "recorded_at_unix": 1_790_000_000
    });
    let approval = fixture.parse(&value).unwrap();
    assert_eq!(approval.headless_mode(), TransportSyncMode::Bidirectional);
    assert_eq!(
        approval.manual_quota(),
        Some(ManualQuota {
            account_limit_bytes: 10_737_418_240,
            used_bytes_manual: 4_294_967_296,
            recorded_at_unix: 1_790_000_000,
        })
    );
}

/// 批准后的能力透传视图根与平面根，且平面根有自己的钉住身份。
#[test]
fn approved_transport_reports_view_and_plane_roots_and_the_new_approved_values() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    let approved = fixture.approve(&scope).unwrap();
    let roots = approved.roots();
    assert_eq!(roots.mirror_view_root(), fixture.root.join("mirror-view"));
    assert_eq!(roots.plane_root(), fixture.root.join("plane"));
    assert_ne!(roots.plane_identity(), roots.private_identity());
    assert_ne!(roots.plane_identity(), roots.mirror_identity());
    assert_eq!(approved.excluded_folders(), ["School", "日记"]);
    assert_eq!(approved.device_name(), "studiolo-fixture");
    assert_eq!(approved.headless_mode(), TransportSyncMode::MirrorRemote);
    assert!(approved.manual_quota().is_none());
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
