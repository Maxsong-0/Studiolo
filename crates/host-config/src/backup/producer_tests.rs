//! 生产者部署事实的失败契约：缺省关闭、账号和路径互不混用、保留期和冻结预算有界。

use super::*;

fn settings() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "worker_uid": 2101,
        "reader_gid": 2102,
        "socket": "/run/studiolo-backup/producer.sock",
        "staging_parent": "/var/lib/studiolo-backup-staging",
        "ephemeral_root": "/var/lib/studiolo-ephemeral",
        "source_retention_millis": 21_600_000,
        "freeze_budget_millis": 30_000,
        "application_packages": [
            { "name": "studiolo-local-server", "version": "0.1.0", "digest": "ab".repeat(32) }
        ],
        "restic_digest": "cd".repeat(32),
        "deployment": [ { "unit": "studiolo-human.service", "revision": "81d0293" } ]
    })
}

fn parse(value: serde_json::Value) -> Result<Option<BackupProducerSettings>, serde_json::Error> {
    decode(value)
}

#[test]
fn a_complete_section_parses_into_closed_settings() {
    let parsed = parse(settings()).expect("decode").expect("settings");

    assert_eq!(parsed.worker_uid(), 2101);
    assert_eq!(parsed.reader_gid(), 2102);
    assert_eq!(
        parsed.socket(),
        Path::new("/run/studiolo-backup/producer.sock")
    );
    assert_eq!(parsed.source_retention_millis(), 21_600_000);
    assert_eq!(parsed.freeze_budget_millis(), 30_000);
    assert_eq!(
        parsed.application_packages()[0].name(),
        "studiolo-local-server"
    );
    assert_eq!(parsed.deployment()[0].revision(), "81d0293");
    assert_eq!(parsed.restic_digest(), "cd".repeat(32));
}

#[test]
fn root_or_sentinel_accounts_are_refused() {
    for (key, bad) in [
        ("worker_uid", 0_u32),
        ("worker_uid", u32::MAX),
        ("reader_gid", 0),
        ("reader_gid", u32::MAX),
    ] {
        let mut value = settings();
        value[key] = serde_json::json!(bad);
        assert!(parse(value).is_err(), "{key}={bad}");
    }
}

#[test]
fn paths_must_be_absolute_normal_and_not_nested_in_each_other() {
    for (key, bad) in [
        ("socket", "run/producer.sock"),
        ("socket", "/run/../etc/producer.sock"),
        ("socket", "/"),
        ("staging_parent", "/var/lib//staging"),
        (
            "ephemeral_root",
            "/var/lib/studiolo-backup-staging/ephemeral",
        ),
        ("socket", "/var/lib/studiolo-backup-staging/producer.sock"),
        ("staging_parent", "/var/lib/studiolo-ephemeral"),
    ] {
        let mut value = settings();
        value[key] = serde_json::json!(bad);
        assert!(parse(value).is_err(), "{key}={bad}");
    }
    let mut long = settings();
    long["socket"] = serde_json::json!(format!("/run/{}/producer.sock", "s".repeat(100)));
    assert!(parse(long).is_err(), "socket 路径过长");
}

#[test]
fn retention_and_freeze_budget_are_bounded() {
    for (key, bad) in [
        ("source_retention_millis", 599_999_u64),
        ("source_retention_millis", 86_400_001),
        ("freeze_budget_millis", 999),
        ("freeze_budget_millis", 120_001),
    ] {
        let mut value = settings();
        value[key] = serde_json::json!(bad);
        assert!(parse(value).is_err(), "{key}={bad}");
    }
}

#[test]
fn package_and_deployment_entries_need_fixed_shapes() {
    for (pointer, bad) in [
        ("/application_packages", serde_json::json!([])),
        (
            "/application_packages/0/digest",
            serde_json::json!("AB".repeat(32)),
        ),
        ("/application_packages/0/digest", serde_json::json!("abcd")),
        ("/application_packages/0/name", serde_json::json!("")),
        (
            "/application_packages/0/version",
            serde_json::json!("0.1 0"),
        ),
        ("/restic_digest", serde_json::json!("zz".repeat(32))),
        ("/deployment", serde_json::json!([])),
        ("/deployment/0/unit", serde_json::json!("a/b.service")),
    ] {
        let mut value = settings();
        *value.pointer_mut(pointer).expect("pointer") = bad.clone();
        assert!(parse(value).is_err(), "{pointer}={bad}");
    }
    let mut many = settings();
    many["deployment"] = serde_json::json!(
        (0..17)
            .map(|index| serde_json::json!({ "unit": format!("u{index}"), "revision": "r" }))
            .collect::<Vec<_>>()
    );
    assert!(parse(many).is_err(), "条目数有上限");
}

#[test]
fn an_unknown_field_or_version_fails_closed() {
    let mut extra = settings();
    extra["repository"] = serde_json::json!("rest:https://backup.example.test/");
    assert!(parse(extra).is_err(), "地址不属于这一段");

    let mut version = settings();
    version["version"] = serde_json::json!(2);
    assert!(parse(version).is_err());

    assert!(parse(serde_json::json!([])).is_err(), "只接受对象");
}
