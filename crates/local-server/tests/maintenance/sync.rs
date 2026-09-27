//! `sync-init` 契约与两条 Bridge 维护命令共用的夹具：既有 Broker 租约下显式初始化四项私有存储、
//! 任一存在即拒绝。残留清除（`sync-purge-residue`）的契约在 `sync_purge.rs`。
//!
//! 依赖 W4 的 `PlaneStateStore::{open,save}` 与 `StageStore::initialize` 真实实现；
//! 在 A 轮桩（一律 `Storage`）上，涉及外 stage 与 plane-state 的用例会以 `SyncStorage` 变红。

use super::{
    super::{
        MaintenanceError,
        inputs::Inputs,
        options::{Command, Options},
        workflow::execute,
    },
    fixture::Fixture,
};
use crate::sync_plane::recovery::PlaneStateStore;
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use studiolo_core::VaultUuid;
use studiolo_vault_broker::MaintenanceActor;

fn accept(_: &Value, _: &str) -> Result<(), MaintenanceError> {
    Ok(())
}
fn check() -> Result<(), MaintenanceError> {
    Ok(())
}

pub(super) fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

pub(super) struct Sync {
    fixture: Fixture,
    approval: PathBuf,
}

impl Sync {
    /// 既有写者存储、权威审计账本加四个 0700 自有根；`with_scope` 决定私有配置是否批准同步范围。
    pub(super) fn new(with_scope: bool) -> Self {
        Self::build(with_scope, true)
    }

    /// 同上但没有权威审计账本（`Inputs::audit` 为 `None`）。
    pub(super) fn without_audit() -> Self {
        Self::build(true, false)
    }

    fn build(with_scope: bool, audit: bool) -> Self {
        let fixture = Fixture::new(audit, true);
        for name in ["transport", "mirror", "mirror-view", "plane"] {
            private_dir(&fixture.root.join(name));
        }
        if with_scope {
            let mut value: Value =
                serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
            value["sync_scope"] = json!({
                "version": 1, "vault_uuid": VaultUuid::new(),
                "link_id": "11111111111111111111111111111111",
                "remote_vault": "22222222222222222222222222222222", "scope_epoch": 1,
                "whole_folders": [{"prefix":"笔记","markdown":true,"attachment_types":[],"max_bytes":1048576}],
                "whole_files": [], "generated_documents": []
            });
            fs::write(&fixture.config, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        let approval = fixture.root.join("config/sync-approval.json");
        let root = &fixture.root;
        let value = json!({
            "schema_version": 1, "enabled": true,
            "private_root": root.join("transport"), "mirror_root": root.join("mirror"),
            "owner_view_root": root.join("mirror-view"), "plane_root": root.join("plane"),
            "device_name": "studiolo-maintain-fixture", "headless_mode": "mirror-remote",
            "excluded_folders": ["日记"], "manual_quota": null,
            "runtime": {"node_path": "/opt/studiolo/node/bin/node",
                "cli_path": "/opt/studiolo/obsidian-headless/cli.js",
                "version": "0.0.14", "closure_digest": "1".repeat(64)},
            "backend": {"service_unit": "studiolo-sync-launcher@link-1.service",
                "launcher_path": "/opt/studiolo/libexec/studiolo-sync-launcher",
                "payload_cgroup_prefix": "/sys/fs/cgroup/studiolo.slice/payload",
                "payload_uid": 61001, "payload_gid": 61001},
            "budget": {"deadline_seconds": 3600, "stop_grace_seconds": 30, "stdout_bytes": 262144,
                "stderr_bytes": 262144, "config_json_bytes": 65536, "private_state_bytes": 67108864,
                "log_bytes": 268435456, "memory_bytes": 2147483648u64, "swap_bytes": 0,
                "pids_max": 64, "cpu_max_percent": 200}
        });
        fs::write(&approval, serde_json::to_vec(&value).unwrap()).unwrap();
        fs::set_permissions(&approval, fs::Permissions::from_mode(0o600)).unwrap();
        Self { fixture, approval }
    }

    fn options(&self, command: Command, folder: Option<&str>) -> Options {
        let mut options = self.fixture.options(command);
        options.sync_transport = Some(self.approval.clone());
        options.folder = folder.map(str::to_owned);
        options
    }

    pub(super) fn run(
        &self,
        command: Command,
        folder: Option<&str>,
    ) -> Result<Value, MaintenanceError> {
        let options = self.options(command, folder);
        let mut inputs = Inputs::open(&options)?;
        execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            accept,
            check,
        )
    }

    pub(super) fn root(&self) -> &Path {
        &self.fixture.root
    }

    pub(super) fn view(&self) -> PathBuf {
        self.root().join("mirror-view")
    }

    /// 守卫拦住那一轮之后的持久状态：`folders` 既是上次核验通过的排除项，也是 headless 仍排除的。
    pub(super) fn mark_verified_excluded(&self, folders: &[&str]) {
        self.mark_plane_state(folders, folders);
    }

    /// 分别写上次核验集合与 headless 最近一次核对报的排除集合（W4 的持久状态 API）。
    pub(super) fn mark_plane_state(&self, verified: &[&str], headless: &[&str]) {
        let plane = self.root().join("plane");
        let mut store = PlaneStateStore::open(&plane).unwrap();
        let mut state = store.state().clone();
        state.verified_excluded = owned(verified);
        state.headless_excluded = owned(headless);
        store.save(state).unwrap();
    }
}

#[test]
fn sync_init_builds_all_four_stores_once_then_refuses_a_second_run() {
    let s = Sync::new(true);
    let report = s.run(Command::SyncInit, None).unwrap();
    assert_eq!(report["command"], "sync-init");
    for item in ["transport", "bridge", "stage", "plane_state"] {
        assert_eq!(report["initialized"][item], true, "{item}");
    }
    assert!(s.root().join("transport/state.json").is_file());
    assert!(s.root().join("plane/bridge/bridge.sqlite3").is_file());
    assert!(s.root().join("plane/stage").is_dir());
    assert!(s.root().join("plane/plane-state.json").is_file());
    assert!(matches!(
        s.run(Command::SyncInit, None),
        Err(MaintenanceError::SyncExists)
    ));
}

#[test]
fn sync_init_refuses_without_an_approved_scope_and_creates_nothing() {
    let s = Sync::new(false);
    assert!(matches!(
        s.run(Command::SyncInit, None),
        Err(MaintenanceError::Configuration)
    ));
    for name in ["transport", "plane", "mirror-view"] {
        assert_eq!(
            fs::read_dir(s.root().join(name)).unwrap().count(),
            0,
            "{name}"
        );
    }
}

#[test]
fn sync_init_refuses_when_any_store_already_exists_before_touching_the_others() {
    let s = Sync::new(true);
    private_dir(&s.root().join("plane/stage"));
    assert!(matches!(
        s.run(Command::SyncInit, None),
        Err(MaintenanceError::SyncExists)
    ));
    assert_eq!(fs::read_dir(s.root().join("transport")).unwrap().count(), 0);
    assert!(!s.root().join("plane/bridge").exists());
    assert!(!s.root().join("plane/plane-state.json").exists());
}

#[test]
fn sync_init_refuses_while_another_owner_holds_the_writer_lease() {
    let s = Sync::new(true);
    let owner = studiolo_vault_broker::FsVaultBroker::open_existing(
        &s.root().join("vault"),
        &s.root().join("data"),
    )
    .unwrap();
    assert!(matches!(
        s.run(Command::SyncInit, None),
        Err(MaintenanceError::Broker)
    ));
    drop(owner);
    assert_eq!(fs::read_dir(s.root().join("transport")).unwrap().count(), 0);
}
