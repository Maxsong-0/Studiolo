//! 只在本次 /tmp 建合成私有根与镜像根；不安装 Headless、不启动进程、不写任何真实密钥。

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use studiolo_core::VaultUuid;
use studiolo_host_config::{
    Config, SyncScope,
    sync_transport::{ApprovedTransport, SyncTransportApproval},
};

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub path: PathBuf,
    pub value: Value,
}

pub fn directory(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_path_buf();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for name in [
            "config",
            "auth",
            "audit",
            "transport",
            "mirror",
            "canonical",
        ] {
            directory(&root.join(name));
        }
        let vault = VaultUuid::new();
        let value = json!({"version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture-audience",
                "subject":"fictional-person","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"logical-seed-id",
            "sync_scope":{"version":1,"vault_uuid":vault,"link_id":"11111111111111111111111111111111",
                "remote_vault":"22222222222222222222222222222222","scope_epoch":1,
                "whole_folders":[{"prefix":"笔记","markdown":true,"attachment_types":[],"max_bytes":1048576}],
                "whole_files":[],"generated_documents":[]}});
        let fixture = Self {
            path: root.join("config/owner.json"),
            root,
            value,
            _temp: temp,
        };
        fixture.write(&fixture.value);
        fixture
    }

    pub fn write(&self, value: &Value) {
        fs::write(&self.path, serde_json::to_vec(value).unwrap()).unwrap();
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    pub fn scope(&self) -> SyncScope {
        Config::load(&self.path)
            .unwrap()
            .sync_scope()
            .unwrap()
            .unwrap()
    }

    /// 形式合法的审批文档；批准值不代表本机已经具备 systemd/cgroup 特权。
    pub fn approval(&self) -> Value {
        json!({
            "schema_version": 1,
            "enabled": true,
            "private_root": self.root.join("transport"),
            "mirror_root": self.root.join("mirror"),
            "runtime": {
                "node_path": "/opt/studiolo/node/bin/node",
                "cli_path": "/opt/studiolo/obsidian-headless/cli.js",
                "version": "0.0.14",
                "closure_digest": "1".repeat(64),
            },
            "backend": {
                "service_unit": "studiolo-sync-launcher@link-1.service",
                "launcher_path": "/opt/studiolo/libexec/studiolo-sync-launcher",
                "payload_cgroup_prefix": "/sys/fs/cgroup/studiolo.slice/payload",
                "payload_uid": 61001,
                "payload_gid": 61001,
            },
            "budget": budget(),
        })
    }

    pub fn parse(
        &self,
        value: &Value,
    ) -> Result<SyncTransportApproval, studiolo_host_config::ConfigError> {
        SyncTransportApproval::parse(&serde_json::to_vec(value).unwrap())
    }

    pub fn approve<'scope>(
        &self,
        scope: &'scope SyncScope,
    ) -> Result<ApprovedTransport<'scope>, studiolo_host_config::ConfigError> {
        let approval = self.parse(&self.approval())?;
        ApprovedTransport::approve(scope, approval, &[&self.root.join("canonical")])
    }
}

pub fn budget() -> Value {
    json!({
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
    })
}
