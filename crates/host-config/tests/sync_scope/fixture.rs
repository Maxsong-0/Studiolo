use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_host_config::{
    Config, ConfigError, SyncContentMetadata, SyncFileKind, SyncPathEvidence, SyncScope,
};

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub path: PathBuf,
    pub value: Value,
    pub vault: VaultUuid,
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
        for name in ["config", "auth", "audit"] {
            directory(&root.join(name));
        }
        let vault = VaultUuid::new();
        let value = json!({"version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture-audience",
                "subject":"fictional-person","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"logical-seed-id",
            "sync_scope":{"version":1,"vault_uuid":vault,"link_id":"11111111111111111111111111111111",
                "remote_vault":"22222222222222222222222222222222","scope_epoch":1,
                "whole_folders":[folder("笔记")],"whole_files":[file("附件/手册",false,&["pdf"])],
                "generated_documents":["笔记/自动汇总.md"]}});
        let fixture = Self {
            path: root.join("config/owner.json"),
            root,
            value,
            vault,
            _temp: temp,
        };
        fixture.write();
        fixture
    }

    pub fn bytes(&self, bytes: &[u8]) {
        fs::write(&self.path, bytes).unwrap();
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    pub fn write(&self) {
        self.write_value(&self.value);
    }

    pub fn write_value(&self, value: &Value) {
        self.bytes(&serde_json::to_vec(value).unwrap());
    }

    pub fn scope(&self) -> SyncScope {
        Config::load(&self.path)
            .unwrap()
            .sync_scope()
            .unwrap()
            .unwrap()
    }

    pub fn load(&self) -> Result<Config, ConfigError> {
        Config::load(&self.path)
    }
}

pub fn folder(prefix: &str) -> Value {
    json!({"prefix":prefix,"markdown":true,"attachment_types":[],"max_bytes":1048576})
}

pub fn file(path: &str, markdown: bool, attachments: &[&str]) -> Value {
    json!({"path":path,"markdown":markdown,"attachment_types":attachments,"max_bytes":1048576})
}

pub fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

pub fn missing(value: &str) -> SyncPathEvidence {
    SyncPathEvidence {
        path: path(value),
        content: None,
    }
}

pub fn present(value: &str, kind: SyncFileKind, bytes: u64) -> SyncPathEvidence {
    SyncPathEvidence {
        path: path(value),
        content: Some(SyncContentMetadata { kind, bytes }),
    }
}

pub fn markdown(value: &str) -> SyncPathEvidence {
    present(value, SyncFileKind::Markdown, 20)
}
