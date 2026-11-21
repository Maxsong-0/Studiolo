//! 给了备份配置却读不出时拒绝启动：不能悄悄当成「没启用」，带着没有备份的服务起来（T-216）。

use super::writer::writer_options;
use super::{Fixture, StartupError, prepare_with, test_support};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, sync::Arc};

/// 内容合格的备份配置（两段成对）；路径都在夹具目录里，不和 Auth／Audit 私有目录互相嵌套。
pub(super) fn approved(root: &Path) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "backup": {
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
        },
        "backup_producer": {
            "version": 1,
            "worker_uid": 2101,
            "reader_gid": 2102,
            // 这一层只装配、不监听，socket 路径不会被打开。
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
        }
    }))
    .unwrap()
}

/// 用同一组启动参数装配一次。装配成功就回报备份生产者装上没有，并把写者放掉，下一轮才能接着装配。
async fn assemble(f: &Fixture, data: &Path, backup: Option<&Path>) -> Result<bool, StartupError> {
    let mut options = f.options();
    options.writer_data = Some(data.to_owned());
    options.backup_config = backup.map(Path::to_owned);
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) }).await;
    f.unbound();
    let prepared = prepared?;
    let enabled = prepared.backup.is_some();
    let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
    drop(prepared);
    workbench.stop();
    workbench.shutdown().await.unwrap();
    Ok(enabled)
}

#[tokio::test]
async fn a_backup_config_that_cannot_be_read_refuses_startup_instead_of_running_without_backup() {
    let f = Fixture::new(true);
    let (_, data) = writer_options(&f);
    let path = f.root.join("config/backup.json");
    let write = |bytes: &[u8], mode: u32| {
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    };
    let refused =
        |result: Result<bool, StartupError>| matches!(result, Err(StartupError::Configuration));

    // 给了路径、文件不存在：拒绝，不当成「没启用」。
    assert!(refused(assemble(&f, &data, Some(&path)).await));
    // 内容合格、只有本人读得到：照常装配，备份生产者装上了。下面两种拒绝各只和这一份差一处。
    write(&approved(&f.root), 0o600);
    assert!(matches!(assemble(&f, &data, Some(&path)).await, Ok(true)));
    // 同一份内容，别人读得到（0644）：拒绝。
    write(&approved(&f.root), 0o644);
    assert!(refused(assemble(&f, &data, Some(&path)).await));
    // 权限合格，读得到但两段都没有：拒绝。
    write(b"{}", 0o600);
    assert!(refused(assemble(&f, &data, Some(&path)).await));
    // 不给备份配置：照常装配、没有备份生产者（那份读不出的文件还留在原地，也不去读）。
    assert!(matches!(assemble(&f, &data, None).await, Ok(false)));
}
