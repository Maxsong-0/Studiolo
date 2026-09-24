//! 双向 Bridge 平面的组合根契约：默认关闭；给了审批文档而私有配置没批范围就拒绝；
//! 审批与范围齐备但本机没有受控 launcher 同样失败关闭，且写者已被关停释放。

use super::{Fixture, StartupError, prepare_with, test_support, writer::writer_options};
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use studiolo_core::VaultUuid;
use studiolo_vault_broker::FsVaultBroker;

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

/// 写一份形式合法的审批文档（0600）并建好三个自有根；批准值不代表本机有 systemd／cgroup 特权。
fn approval(f: &Fixture) -> PathBuf {
    for name in ["transport", "mirror", "mirror-view", "plane"] {
        private_dir(&f.root.join(name));
    }
    let path = f.root.join("config/sync-approval.json");
    let value = json!({
        "schema_version": 1,
        "enabled": true,
        "private_root": f.root.join("transport"),
        "mirror_root": f.root.join("mirror"),
        "owner_view_root": f.root.join("mirror-view"),
        "plane_root": f.root.join("plane"),
        "device_name": "studiolo-composition",
        "headless_mode": "mirror-remote",
        "excluded_folders": [],
        "manual_quota": null,
        "runtime": {
            "node_path": "/opt/studiolo/node/bin/node",
            "cli_path": "/opt/studiolo/obsidian-headless/cli.js",
            "version": "0.0.14",
            "closure_digest": "1".repeat(64)
        },
        "backend": {
            "service_unit": "studiolo-sync-launcher@link-1.service",
            "launcher_path": "/opt/studiolo/libexec/studiolo-sync-launcher",
            "payload_cgroup_prefix": "/sys/fs/cgroup/studiolo.slice/payload",
            "payload_uid": 61001,
            "payload_gid": 61001
        },
        "budget": {
            "deadline_seconds": 3600,
            "stop_grace_seconds": 30,
            "stdout_bytes": 262144,
            "stderr_bytes": 262144,
            "config_json_bytes": 65536,
            "private_state_bytes": 67108864,
            "log_bytes": 268435456,
            "memory_bytes": 2147483648u64,
            "swap_bytes": 0,
            "pids_max": 64,
            "cpu_max_percent": 200
        }
    });
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

/// 给私有配置加上范围批准；夹具其余字段不动。
fn approve_scope(f: &Fixture) {
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    value["sync_scope"] = json!({
        "version": 1,
        "vault_uuid": VaultUuid::new(),
        "link_id": "11111111111111111111111111111111",
        "remote_vault": "22222222222222222222222222222222",
        "scope_epoch": 1,
        "whole_folders": [{"prefix":"笔记","markdown":true,"attachment_types":[],"max_bytes":1048576}],
        "whole_files": [],
        "generated_documents": []
    });
    fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
}

/// 默认关闭：没有 `--sync-transport` 就没有平面，写者与既有装配完全一样地起来、一样地关停。
#[tokio::test]
async fn without_sync_transport_the_plane_is_not_assembled_and_the_writer_serves_as_before() {
    let f = Fixture::new(true);
    let (options, data) = writer_options(&f);
    assert!(options.sync_transport.is_none());
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    f.unbound();
    assert!(prepared.sync_plane.is_none());
    let workbench = Arc::clone(prepared.workbench.as_ref().unwrap());
    workbench.stop();
    workbench.shutdown().await.unwrap();
    drop((prepared, workbench));
    drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
}

/// 给了审批文档但私有配置没有 `sync_scope`：不静默降级，按平面类别拒绝并释放写者。
#[tokio::test]
async fn approval_without_an_approved_scope_fails_as_sync_plane_and_releases_the_writer() {
    let f = Fixture::new(true);
    let (mut options, data) = writer_options(&f);
    options.sync_transport = Some(approval(&f));
    let error = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .err();
    assert!(matches!(error, Some(StartupError::SyncPlane)), "{error:?}");
    f.unbound();
    drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
}

/// 审批与范围都齐备、三个自有根也是 0700 私有目录，但本机没有受控 launcher：
/// 仍是 `SyncPlane` 类失败（不是 panic、不是别的类别），既不起第三方程序也不建私有存储，
/// 并且写者已经关停。
#[tokio::test]
async fn approved_scope_without_a_trusted_launcher_fails_closed_and_releases_the_writer() {
    let f = Fixture::new(true);
    let (mut options, data) = writer_options(&f);
    approve_scope(&f);
    options.sync_transport = Some(approval(&f));
    let error = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .err();
    assert!(matches!(error, Some(StartupError::SyncPlane)), "{error:?}");
    f.unbound();
    for name in ["transport", "plane", "mirror-view"] {
        assert_eq!(
            fs::read_dir(f.root.join(name)).unwrap().count(),
            0,
            "{name} 不得被初始化"
        );
    }
    drop(FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap());
}
