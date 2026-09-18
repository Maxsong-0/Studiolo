//! 失败契约：默认关闭，越界/未知/重叠/非私有一律拒绝，且不产生部分能力。

use super::fixture::{Fixture, budget, directory};
use serde_json::json;
use std::fs;
use studiolo_host_config::{ConfigError, sync_transport::SyncTransportApproval};

#[test]
fn approval_without_explicit_enable_produces_no_capability() {
    let fixture = Fixture::new();
    let mut value = fixture.approval();
    value["enabled"] = json!(false);
    assert_eq!(fixture.parse(&value).err(), Some(ConfigError));

    let mut missing = fixture.approval();
    missing.as_object_mut().unwrap().remove("enabled");
    assert_eq!(fixture.parse(&missing).err(), Some(ConfigError));
}

#[test]
fn unknown_fields_wrong_schema_version_or_positional_array_are_rejected() {
    let fixture = Fixture::new();
    let mut unknown = fixture.approval();
    unknown["extra"] = json!(1);
    assert_eq!(fixture.parse(&unknown).err(), Some(ConfigError));

    let mut version = fixture.approval();
    version["schema_version"] = json!(2);
    assert_eq!(fixture.parse(&version).err(), Some(ConfigError));

    assert_eq!(
        SyncTransportApproval::parse(b"[1,2,3]").err(),
        Some(ConfigError)
    );
    assert_eq!(
        SyncTransportApproval::parse(b"{\"schema_version\":1").err(),
        Some(ConfigError)
    );
    let mut trailing = serde_json::to_vec(&fixture.approval()).unwrap();
    trailing.extend_from_slice(b"{}");
    assert_eq!(
        SyncTransportApproval::parse(&trailing).err(),
        Some(ConfigError)
    );
}

#[test]
fn runtime_outside_the_pinned_release_or_with_a_malformed_digest_is_rejected() {
    let fixture = Fixture::new();
    for (pointer, replacement) in [
        ("/runtime/version", json!("0.0.15")),
        ("/runtime/closure_digest", json!("A".repeat(64))),
        ("/runtime/closure_digest", json!("1".repeat(63))),
        ("/runtime/node_path", json!("node")),
        ("/runtime/cli_path", json!("/opt/../etc/cli.js")),
    ] {
        let mut value = fixture.approval();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{pointer}");
    }

    let mut same = fixture.approval();
    same["runtime"]["cli_path"] = same["runtime"]["node_path"].clone();
    assert_eq!(fixture.parse(&same).err(), Some(ConfigError));
}

#[test]
fn payload_identity_of_root_or_the_owner_itself_is_not_isolation() {
    let fixture = Fixture::new();
    let owner = rustix::process::geteuid().as_raw();
    for (pointer, replacement) in [
        ("/backend/payload_uid", json!(0)),
        ("/backend/payload_gid", json!(0)),
        ("/backend/payload_uid", json!(owner)),
    ] {
        let mut value = fixture.approval();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{pointer}");
    }
}

#[test]
fn service_unit_and_payload_cgroup_prefix_outside_the_closed_shape_are_rejected() {
    let fixture = Fixture::new();
    for (pointer, replacement) in [
        ("/backend/service_unit", json!("studiolo-sync.timer")),
        ("/backend/service_unit", json!(".service")),
        ("/backend/service_unit", json!("/etc/x.service")),
        ("/backend/service_unit", json!("bad unit.service")),
        ("/backend/payload_cgroup_prefix", json!("/sys/fs/cgroup/")),
        ("/backend/payload_cgroup_prefix", json!("/tmp/payload")),
        ("/backend/launcher_path", json!("launcher")),
    ] {
        let mut value = fixture.approval();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{pointer}");
    }
}

#[test]
fn budget_out_of_range_or_with_any_swap_allowance_is_rejected() {
    let fixture = Fixture::new();
    for (key, replacement) in [
        ("swap_bytes", json!(1)),
        ("deadline_seconds", json!(0)),
        ("deadline_seconds", json!(86_401)),
        ("stop_grace_seconds", json!(0)),
        ("config_json_bytes", json!(65_537)),
        ("stdout_bytes", json!(0)),
        ("pids_max", json!(0)),
        ("cpu_max_percent", json!(6_401)),
        ("memory_bytes", json!(0)),
        ("log_bytes", json!(0)),
        ("private_state_bytes", json!(0)),
    ] {
        let mut value = fixture.approval();
        value["budget"][key] = replacement;
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{key}");
    }

    let mut missing = fixture.approval();
    let mut partial = budget();
    partial.as_object_mut().unwrap().remove("pids_max");
    missing["budget"] = partial;
    assert_eq!(fixture.parse(&missing).err(), Some(ConfigError));
}

#[test]
fn overlapping_private_mirror_or_reserved_roots_never_yield_a_capability() {
    let fixture = Fixture::new();
    let scope = fixture.scope();

    let mut same = fixture.approval();
    same["mirror_root"] = json!(fixture.root.join("transport"));
    let approval = fixture.parse(&same).unwrap();
    assert!(
        studiolo_host_config::sync_transport::ApprovedTransport::approve(&scope, approval, &[])
            .is_err()
    );

    directory(&fixture.root.join("transport/nested"));
    let mut nested = fixture.approval();
    nested["mirror_root"] = json!(fixture.root.join("transport/nested"));
    let approval = fixture.parse(&nested).unwrap();
    assert!(
        studiolo_host_config::sync_transport::ApprovedTransport::approve(&scope, approval, &[])
            .is_err()
    );

    let approval = fixture.parse(&fixture.approval()).unwrap();
    assert!(
        studiolo_host_config::sync_transport::ApprovedTransport::approve(
            &scope,
            approval,
            &[&fixture.root]
        )
        .is_err()
    );
}

#[test]
fn a_non_private_or_missing_root_directory_never_yields_a_capability() {
    let fixture = Fixture::new();
    let scope = fixture.scope();
    fs::set_permissions(
        fixture.root.join("transport"),
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();
    assert!(fixture.approve(&scope).is_err());

    fs::set_permissions(
        fixture.root.join("transport"),
        <fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .unwrap();
    assert!(fixture.approve(&scope).is_ok());

    fs::remove_dir(fixture.root.join("mirror")).unwrap();
    assert!(fixture.approve(&scope).is_err());
}

#[test]
fn oversized_or_empty_approval_input_is_rejected_before_parsing() {
    let fixture = Fixture::new();
    let mut padded = fixture.approval();
    padded["runtime"]["node_path"] = json!(format!("/opt/{}", "p".repeat(9_000)));
    assert_eq!(fixture.parse(&padded).err(), Some(ConfigError));
    assert_eq!(SyncTransportApproval::parse(b"").err(), Some(ConfigError));
}
