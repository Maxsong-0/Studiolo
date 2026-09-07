//! 真实Broker、双库和watcher启动组合；只替换固定公钥Adapter，不替换来源证明。

use super::super::prepare_composed;
use super::{Fixture, StartupError, prepare_with, test_support};
use crate::options::FactorOptions;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};
use studiolo_vault_broker::{BrokerError, FsVaultBroker};

pub(super) fn writer_options(f: &Fixture) -> (crate::options::Options, PathBuf) {
    let data = f.root.join("writer-data");
    fs::create_dir(&data).unwrap();
    fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
    drop(FsVaultBroker::open(&f.root.join("vault"), &data).unwrap());
    let mut options = f.options();
    options.writer_data = Some(data.clone());
    (options, data)
}

#[tokio::test]
async fn prepared_writer_is_exclusive_and_shutdown_allows_reopen() {
    let f = Fixture::new(true);
    let (options, data) = writer_options(&f);
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    f.unbound();
    assert!(matches!(
        FsVaultBroker::open_existing(&f.root.join("vault"), &data),
        Err(BrokerError::WriterBusy)
    ));
    let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
    let (listener, app) = prepared.bind().await.unwrap();
    assert_eq!(listener.local_addr().unwrap().port(), f.port);
    workbench.stop();
    workbench.shutdown().await.unwrap();
    drop((listener, app, workbench));
    drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
}

#[tokio::test]
async fn old_canonical_mode_does_not_take_or_upgrade_the_writer_lease() {
    let f = Fixture::new(true);
    let (_, data) = writer_options(&f);
    let owner = FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap();
    let mut options = f.options();
    options.canonical = true;
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    assert!(prepared.workbench.is_none());
    assert!(prepared.canonical.is_some());
    drop(prepared);
    let mut options = f.options();
    options.writer_data = Some(data);
    assert!(matches!(
        prepare_with(options, |_| async { panic!("租约争用不得取钥") }).await,
        Err(StartupError::Writer)
    ));
    drop(owner);
    f.unbound();
}

#[tokio::test]
async fn missing_and_wrong_data_never_initialize_a_second_binding() {
    for missing in [false, true] {
        let f = Fixture::new(true);
        let (mut options, _) = writer_options(&f);
        let wrong = f.root.join("other-data");
        if !missing {
            fs::create_dir(&wrong).unwrap();
            fs::set_permissions(&wrong, fs::Permissions::from_mode(0o700)).unwrap();
        }
        options.writer_data = Some(wrong.clone());
        let error = prepare_with(options, |_| async { panic!("无效写者不得取钥") }).await;
        assert!(matches!(
            error,
            Err(StartupError::Writer | StartupError::Configuration)
        ));
        if missing {
            assert!(!wrong.exists());
        } else {
            assert_eq!(fs::read_dir(wrong).unwrap().count(), 0);
        }
        f.unbound();
    }
}

#[tokio::test]
async fn first_capture_failure_does_not_listen_and_releases_its_writer() {
    let f = Fixture::new(true);
    let (options, data) = writer_options(&f);
    std::os::unix::fs::symlink("note.md", f.root.join("vault/link.md")).unwrap();
    assert!(matches!(
        prepare_with(options, |_| async { Ok(test_support::verifier().await) }).await,
        Err(StartupError::Writer)
    ));
    f.unbound();
    drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
}

#[tokio::test]
async fn writer_roots_changed_during_access_initialization_fail_before_listen() {
    let f = Fixture::new(true);
    let (options, data) = writer_options(&f);
    let result = prepare_with(options, |_| async {
        fs::rename(&data, f.root.join("retained-data")).unwrap();
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o700)).unwrap();
        Ok(test_support::verifier().await)
    })
    .await;
    assert!(matches!(result, Err(StartupError::Writer)));
    assert_eq!(fs::read_dir(&data).unwrap().count(), 0);
    f.unbound();
}

#[tokio::test]
async fn final_config_replacement_rejects_writer_binding_and_keeps_port_closed() {
    let f = Fixture::new(true);
    let (options, data) = writer_options(&f);
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
    fs::rename(&f.config, f.root.join("config/old.json")).unwrap();
    fs::copy(f.root.join("config/old.json"), &f.config).unwrap();
    assert!(matches!(
        prepared.bind().await,
        Err(StartupError::Configuration)
    ));
    workbench.stop();
    workbench.shutdown().await.unwrap();
    drop(workbench);
    f.unbound();
    drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
}

#[tokio::test]
async fn factor_socket_cannot_use_writer_data_before_secrets_are_read() {
    let f = Fixture::new(true);
    let (mut options, data) = writer_options(&f);
    options.factor = Some(FactorOptions {
        socket: data.join("http.sock"),
        replacement_seed_ref: None,
    });
    let result = prepare_composed(
        options,
        |_| async { panic!("隔离失败不得取钥") },
        || panic!("隔离失败不得读秘密"),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Ingress)));
    assert!(!data.join("http.sock").exists());
    f.unbound();
}
