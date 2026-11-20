//! 给了备份配置却读不出时拒绝启动：不能悄悄当成「没启用」，带着没有备份的服务起来（T-216）。

use super::writer::writer_options;
use super::{Fixture, StartupError, prepare_with, test_support};
use std::{fs, os::unix::fs::PermissionsExt};

#[tokio::test]
async fn a_backup_config_that_cannot_be_read_refuses_startup_instead_of_running_without_backup() {
    let f = Fixture::new(true);
    let (_, data) = writer_options(&f);
    let path = f.root.join("config/backup.json");
    // 文件不存在、别人读得到（0644）、读得到但两段都没有：三种都拒绝。
    for case in ["missing", "exposed", "empty-object"] {
        if case != "missing" {
            fs::write(&path, b"{}").unwrap();
            let mode = if case == "exposed" { 0o644 } else { 0o600 };
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        }
        let mut options = f.options();
        options.writer_data = Some(data.clone());
        options.backup_config = Some(path.clone());
        let result = prepare_with(options, |_| async { Ok(test_support::verifier().await) }).await;
        assert!(matches!(result, Err(StartupError::Configuration)), "{case}");
        f.unbound();
    }
    // 对照：同一组参数不给备份配置，照常装配起来——上面的拒绝确实出在备份配置上。
    fs::remove_file(&path).unwrap();
    let mut options = f.options();
    options.writer_data = Some(data);
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    assert!(prepared.workbench.is_some());
    f.unbound();
}
