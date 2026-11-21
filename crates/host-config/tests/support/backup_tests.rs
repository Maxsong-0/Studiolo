//! 备份两段的契约：放在单独的私有备份配置文件里、成对才启用，路径不和 Auth／Audit 私有目录互相嵌套；
//! 写进主人配置一律拒绝（主人配置的整份字节是 Bridge `scope_hash` 的输入，T-216）。

use super::*;
use std::os::unix::fs::symlink;
use studiolo_host_config::backup::BackupConfig;

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

/// 成对的备份配置文件内容。
fn both(root: &Path) -> Value {
    json!({ "version": 1, "backup": approval(), "backup_producer": producer(root) })
}

/// 备份配置文件和主人配置放在同一个 0700 目录里，0600。
fn backup_path(fixture: &Fixture) -> PathBuf {
    fixture.root.join("config/backup.json")
}

fn write_backup(fixture: &Fixture, value: &Value) -> PathBuf {
    let path = backup_path(fixture);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn load(
    fixture: &Fixture,
    value: &Value,
) -> Result<BackupConfig, studiolo_host_config::ConfigError> {
    let path = write_backup(fixture, value);
    BackupConfig::load(&path, &Config::load(&fixture.path).unwrap())
}

#[test]
fn backup_sections_in_the_owner_config_are_rejected_so_the_bridge_binding_never_moves() {
    let fixture = Fixture::new();
    for sections in [
        vec!["backup", "backup_producer"],
        vec!["backup"],
        vec!["backup_producer"],
    ] {
        let mut value = fixture.value.clone();
        for section in sections {
            value[section] = both(&fixture.root)[section].clone();
        }
        fixture.rejects(&value);
    }
}

#[test]
fn the_separate_backup_file_needs_both_sections_and_nothing_else() {
    let fixture = Fixture::new();
    let loaded = load(&fixture, &both(&fixture.root)).expect("两段都在才启用");
    assert_eq!(loaded.approval().schedule().interval_millis(), 600_000);
    assert_eq!(loaded.approval().schedule().rpo_millis(), 900_000);
    assert_eq!(loaded.producer().worker_uid(), 2101);
    assert_eq!(
        loaded.producer().staging_parent(),
        fixture.root.join("backup-staging")
    );

    // 只有审批没有部署事实，或者反过来，都拒绝：不留「批了但不知道往哪放」的半开状态。
    for missing in ["backup", "backup_producer", "version"] {
        let mut half = both(&fixture.root);
        half.as_object_mut().unwrap().remove(missing);
        assert!(load(&fixture, &half).is_err(), "缺 {missing}");
    }
    for (key, value) in [
        ("version", json!(2)),
        ("backup", Value::Null),
        ("backup_producer", Value::Null),
        ("sync_scope", json!({})),
    ] {
        let mut changed = both(&fixture.root);
        changed[key] = value;
        assert!(load(&fixture, &changed).is_err(), "{key}");
    }
}

/// 置备脚本（`deploy/backup-worker/provision/prereq.py`）生成的私有备份配置，原样交给这里的真实解析。
///
/// 脚本里那套取值规则是照抄的：抄漏一条，要到停服之后人侧启动失败才知道。同目录的夹具和脚本输出逐字节相同，
/// 由 `deploy/backup-worker/provision/test_provision.py` 钉住（账号和读取组的号码用占位值）。
/// 夹具放在本 crate 里：随包测试编译期读的文件得在发行包的源码白名单内（`scripts/test-agent-package.py`）。
#[test]
fn the_backup_file_the_provisioning_script_generates_is_accepted_byte_for_byte() {
    let fixture = Fixture::new();
    let path = backup_path(&fixture);
    let generated = include_bytes!("provisioned-backup.json");
    fs::write(&path, generated).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let loaded = BackupConfig::load(&path, &Config::load(&fixture.path).unwrap())
        .expect("置备脚本生成的配置");
    let (approval, producer) = (loaded.approval(), loaded.producer());
    assert!(!approval.claims_immutability());
    assert_eq!(approval.schedule().interval_millis(), 600_000);
    assert_eq!(approval.schedule().rpo_millis(), 900_000);
    assert_eq!(approval.credentials().len(), 3);
    assert_eq!(producer.worker_uid(), 64001);
    assert_eq!(producer.reader_gid(), 64002);
    assert_eq!(
        producer.socket(),
        Path::new("/run/studiolo-backup/producer.sock")
    );
    assert_eq!(
        producer.staging_parent(),
        Path::new("/var/lib/studiolo-backup-staging")
    );
    assert_eq!(producer.source_retention_millis(), 3_600_000);
    assert_eq!(producer.freeze_budget_millis(), 30_000);
    assert_eq!(producer.application_packages().len(), 3);
    assert_eq!(producer.deployment().len(), 2);
}

#[test]
fn the_separate_backup_file_is_read_with_the_same_private_file_rules() {
    let fixture = Fixture::new();
    let path = write_backup(&fixture, &both(&fixture.root));
    let owner = Config::load(&fixture.path).unwrap();
    for mode in [0o640, 0o644, 0o660] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert!(BackupConfig::load(&path, &owner).is_err(), "{mode:o}");
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_ok());
    // 不是 JSON 对象、超过 16 KiB 都拒绝。
    fs::write(&path, b"[]").unwrap();
    assert!(BackupConfig::load(&path, &owner).is_err());
    fs::write(&path, vec![b' '; 16_385]).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_err());
}

#[test]
fn a_missing_linked_or_exposed_backup_file_is_rejected_and_never_taken_as_not_enabled() {
    let fixture = Fixture::new();
    let path = write_backup(&fixture, &both(&fixture.root));
    let owner = Config::load(&fixture.path).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_ok());

    // 相对路径一律拒绝。这一条单独证明不了是哪条规则拦的（当前目录下本来也没有这个文件）；
    // `--backup-config` 给相对路径在选项层另有测试（local-server 的 `a_relative_backup_config_path_is_rejected`）。
    assert!(BackupConfig::load(Path::new("config/backup.json"), &owner).is_err());

    // 末级是符号链接：指向的那份内容和权限都合格也拒绝。
    let real = fixture.root.join("config/backup-real.json");
    fs::rename(&path, &real).unwrap();
    symlink(&real, &path).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_err());
    fs::remove_file(&path).unwrap();

    // 文件不存在：给了路径却读不到是错误，不能当成「没启用」。
    assert!(BackupConfig::load(&path, &owner).is_err());
    fs::rename(&real, &path).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_ok());

    // 父目录不是 0700。
    let parent = fixture.root.join("config");
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_err());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(BackupConfig::load(&path, &owner).is_ok());
}

#[test]
fn backup_paths_may_not_nest_with_the_private_auth_or_audit_directories() {
    let fixture = Fixture::new();
    for (key, inside) in [
        ("socket", "auth/producer.sock"),
        ("staging_parent", "audit/staging"),
        ("ephemeral_root", "auth/ephemeral"),
    ] {
        let mut value = both(&fixture.root);
        value["backup_producer"][key] = json!(fixture.root.join(inside));
        assert!(load(&fixture, &value).is_err(), "{key}");
    }
    // 反方向：封存父目录是私有目录的上级。
    let mut value = both(&fixture.root);
    value["backup_producer"]["staging_parent"] = json!(fixture.root);
    assert!(load(&fixture, &value).is_err());
}
