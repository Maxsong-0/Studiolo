//! 最终监听前仓库路径绑定复核与会话隔离拒绝。

use super::{Fixture, StartupError, prepare_with};

#[tokio::test]
async fn repository_replacement_after_prepare_prevents_listen() {
    let f = Fixture::new(true);
    let repository = tempfile::tempdir().unwrap();
    let mut options = f.options();
    options.session = None;
    options.project_repository = Some(("项目/虚构".into(), repository.path().to_owned()));
    let prepared = prepare_with(options, |_| async {
        panic!("本机入口不得取外部密钥")
    })
    .await
    .unwrap();
    let moved = f.root.join("moved-repository");
    std::fs::rename(repository.path(), moved).unwrap();
    std::fs::create_dir(repository.path()).unwrap();
    assert!(matches!(
        prepared.bind().await,
        Err(StartupError::ReadRoots)
    ));
    f.unbound();
}

#[tokio::test]
async fn manually_composed_session_cannot_bypass_repository_mode_restriction() {
    let f = Fixture::new(true);
    let mut options = f.options();
    options.project_repository = Some(("项目/虚构".into(), f.root.join("must-not-open")));
    let result = prepare_with(options, |_| async {
        panic!("无效组合不得取外部密钥")
    })
    .await;
    assert!(matches!(result, Err(StartupError::Configuration)));
    f.unbound();
}

#[tokio::test]
async fn manually_enabled_cargo_without_binding_fails_before_opening_inputs() {
    let f = Fixture::new(true);
    let mut options = f.options();
    options.session = None;
    options.cargo_dependencies = true;
    options.vault = f.root.join("must-not-open");
    let result = prepare_with(options, |_| async {
        panic!("无效组合不得取外部密钥")
    })
    .await;
    assert!(matches!(result, Err(StartupError::Configuration)));
    assert!(!f.root.join("must-not-open").exists());
    f.unbound();
}
