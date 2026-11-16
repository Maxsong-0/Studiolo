//! 备份的两段配置在整份私有配置里的契约：成对才启用，路径不和 Auth／Audit 私有目录互相嵌套。

use super::*;

fn approval() -> Value {
    json!({
        "version": 1,
        "target_shape": "dedicated_append_only_target",
        "claims_immutability": true,
        "restic_package": "restic",
        "restic_version": "0.19.1",
        "schedule_interval_millis": 600_000,
        "completion_budget_millis": 300_000,
        "rpo_millis": 900_000,
        "keep_last": 30,
        "keep_daily": 14,
        "credentials": [
            { "logical_name": "restic_repository_password",
              "environment_variable": "RESTIC_PASSWORD_FILE" }
        ]
    })
}

fn producer(root: &Path) -> Value {
    json!({
        "version": 1,
        "worker_uid": 2101,
        "reader_gid": 2102,
        // socket 路径有长度上限，临时目录可能很深：正例用固定的短路径（这一层不打开它）。
        "socket": "/run/studiolo-backup/producer.sock",
        "staging_parent": root.join("backup-staging"),
        "ephemeral_root": root.join("ephemeral"),
        "source_retention_millis": 21_600_000,
        "freeze_budget_millis": 30_000,
        "application_packages": [
            { "name": "studiolo-local-server", "version": "0.1.0", "digest": "ab".repeat(32) }
        ],
        "restic_digest": "cd".repeat(32),
        "deployment": [ { "unit": "studiolo-human.service", "revision": "fixture-1" } ]
    })
}

#[test]
fn backup_is_off_by_default_and_needs_both_sections_to_turn_on() {
    let fixture = Fixture::new();
    assert!(
        Config::load(&fixture.path)
            .unwrap()
            .backup_producer()
            .is_none()
    );

    let mut both = fixture.value.clone();
    both["backup"] = approval();
    both["backup_producer"] = producer(&fixture.root);
    fixture.write(&both);
    let config = Config::load(&fixture.path).unwrap();
    let (approved, settings) = config.backup_producer().expect("两段都在才启用");
    assert_eq!(approved.schedule().interval_millis(), 600_000);
    assert_eq!(settings.worker_uid(), 2101);
    assert_eq!(
        settings.staging_parent(),
        fixture.root.join("backup-staging")
    );

    // 只有审批没有部署事实，或者反过来，都在加载时拒绝：不留「批了但不知道往哪放」的半开状态。
    for missing in ["backup", "backup_producer"] {
        let mut half = both.clone();
        half.as_object_mut().unwrap().remove(missing);
        fixture.rejects(&half);
    }
}

#[test]
fn backup_paths_may_not_nest_with_the_private_auth_or_audit_directories() {
    let fixture = Fixture::new();
    for (key, inside) in [
        ("socket", "auth/producer.sock"),
        ("staging_parent", "audit/staging"),
        ("ephemeral_root", "auth/ephemeral"),
    ] {
        let mut value = fixture.value.clone();
        value["backup"] = approval();
        value["backup_producer"] = producer(&fixture.root);
        value["backup_producer"][key] = json!(fixture.root.join(inside));
        fixture.rejects(&value);
    }
    // 反方向：封存父目录是私有目录的上级。
    let mut value = fixture.value.clone();
    value["backup"] = approval();
    value["backup_producer"] = producer(&fixture.root);
    value["backup_producer"]["staging_parent"] = json!(fixture.root);
    fixture.rejects(&value);
}
