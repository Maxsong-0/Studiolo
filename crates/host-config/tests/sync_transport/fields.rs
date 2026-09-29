//! 新字段契约（T-132 P1）：视图根、平面根、设备名、模式、排除项、人工配额与 0600 读取；
//! 缺失、越界或非私有一律拒绝，且不产生部分能力。

use super::fixture::{Fixture, directory};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_host_config::{ConfigError, sync_transport::SyncTransportApproval};

#[test]
fn every_new_field_must_be_present_even_when_its_value_is_null() {
    let fixture = Fixture::new();
    for field in [
        "owner_view_root",
        "plane_root",
        "device_name",
        "headless_mode",
        "excluded_folders",
        "manual_quota",
    ] {
        let mut value = fixture.approval();
        value.as_object_mut().unwrap().remove(field);
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{field}");
    }
    // 显式 `null` 是「配额未知」的申报，与漏写整个键不同。
    let mut explicit = fixture.approval();
    explicit["manual_quota"] = json!(null);
    assert!(fixture.parse(&explicit).unwrap().manual_quota().is_none());
}

#[test]
fn headless_mode_accepts_only_the_two_approved_literals() {
    let fixture = Fixture::new();
    for mode in ["pull-only", "", "Mirror-Remote", "bidirectional ", "sync"] {
        let mut value = fixture.approval();
        value["headless_mode"] = json!(mode);
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{mode:?}");
    }
    for mode in ["mirror-remote", "bidirectional"] {
        let mut value = fixture.approval();
        value["headless_mode"] = json!(mode);
        assert!(fixture.parse(&value).is_ok(), "{mode}");
    }
}

#[test]
fn excluded_folders_reject_absolute_dotted_duplicate_or_oversized_lists() {
    let fixture = Fixture::new();
    let oversized: Vec<String> = (0..65).map(|index| format!("f{index}")).collect();
    for (label, folders) in [
        ("leading slash", json!(["/School"])),
        ("trailing slash", json!(["School/"])),
        ("parent segment", json!(["School/../日记"])),
        ("dot segment", json!(["./School"])),
        ("empty segment", json!(["School//子目录"])),
        ("backslash", json!(["School\\子目录"])),
        ("blank", json!([" "])),
        ("control", json!(["School\n"])),
        ("duplicate", json!(["School", "School"])),
        ("oversized", json!(oversized)),
    ] {
        let mut value = fixture.approval();
        value["excluded_folders"] = folders;
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{label}");
    }
    let mut empty = fixture.approval();
    empty["excluded_folders"] = json!([]);
    assert!(fixture.parse(&empty).unwrap().excluded_folders().is_empty());
}

#[test]
fn manual_quota_must_be_a_consistent_positive_record() {
    let fixture = Fixture::new();
    for (label, quota) in [
        (
            "used above limit",
            json!({"account_limit_bytes": 10, "used_bytes_manual": 11, "recorded_at_unix": 1}),
        ),
        (
            "zero limit",
            json!({"account_limit_bytes": 0, "used_bytes_manual": 0, "recorded_at_unix": 1}),
        ),
        (
            "unset time",
            json!({"account_limit_bytes": 10, "used_bytes_manual": 1, "recorded_at_unix": 0}),
        ),
        (
            "missing field",
            json!({"account_limit_bytes": 10, "used_bytes_manual": 1}),
        ),
        (
            "unknown field",
            json!({"account_limit_bytes": 10, "used_bytes_manual": 1, "recorded_at_unix": 1, "plan": "x"}),
        ),
    ] {
        let mut value = fixture.approval();
        value["manual_quota"] = quota;
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{label}");
    }
}

#[test]
fn device_name_must_fit_a_conflict_copy_name() {
    let fixture = Fixture::new();
    let overlong = "n".repeat(65);
    let wide = "镜".repeat(22);
    let rejected = [
        "",
        "   ",
        "\t",
        "studiolo\nmirror",
        overlong.as_str(),
        // 按字节计：22 个汉字是 66 字节。
        wide.as_str(),
        "studiolo (mirror)",
        "studiolo)",
        "studiolo/mirror",
        // 与 Studiolo 自己的副本设备名同名或只差大小写。
        "studiolo-canonical",
        "Studiolo-Canonical",
    ];
    for name in rejected {
        let mut value = fixture.approval();
        value["device_name"] = json!(name);
        assert_eq!(fixture.parse(&value).err(), Some(ConfigError), "{name:?}");
    }
    // 上限 64 字节：64 个 ASCII、21 个汉字都可以；设备名可以带空格（副本名解析允许）。
    let longest = "n".repeat(64);
    let widest = "镜".repeat(21);
    for name in [longest.as_str(), widest.as_str(), "studiolo 169 mirror"] {
        let mut value = fixture.approval();
        value["device_name"] = json!(name);
        assert!(fixture.parse(&value).is_ok(), "{name:?}");
    }
}

#[test]
fn any_two_of_the_four_roots_sharing_a_path_are_rejected_at_parse_time() {
    let fixture = Fixture::new();
    let roots = [
        "private_root",
        "mirror_root",
        "owner_view_root",
        "plane_root",
    ];
    for (index, left) in roots.iter().enumerate() {
        for right in &roots[index + 1..] {
            let mut value = fixture.approval();
            value[*right] = value[*left].clone();
            assert_eq!(
                fixture.parse(&value).err(),
                Some(ConfigError),
                "{left}={right}"
            );
        }
    }
}

/// 读取走私有配置同一读法：文件 0600、父目录 0700、本人所有；任何放宽都不产生能力。
#[test]
fn load_accepts_only_a_private_file_inside_a_private_directory() {
    let fixture = Fixture::new();
    let parent = fixture.root.join("approval");
    directory(&parent);
    let path = parent.join("sync-approval.json");
    fs::write(&path, serde_json::to_vec(&fixture.approval()).unwrap()).unwrap();

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(SyncTransportApproval::load(&path).is_err());

    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        SyncTransportApproval::load(&path).unwrap().device_name(),
        "studiolo-fixture"
    );

    fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(SyncTransportApproval::load(&path).is_err());

    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(SyncTransportApproval::load(&path).is_ok());
}
