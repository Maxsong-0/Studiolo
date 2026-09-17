use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_host_config::{Config, FileOperationPolicy};

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub path: PathBuf,
    pub value: Value,
    pub vault: VaultUuid,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_path_buf();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["config", "auth", "audit"] {
            let path = root.join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let vault = VaultUuid::new();
        let value = json!({
            "version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture",
                "subject":"fictional-person","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"logical-seed-id",
            "human_content":{"version":1,"vault_uuid":vault,"folders":["notes","other"],
                "protected":["blocked"],"documents":{"one.md":[],"two.md":[],
                    "generated/auto.md":[["<!-- start -->","<!-- end -->"]]}},
            "file_operations":{"version":1,"vault_uuid":vault,"scope_epoch":7,
                "effective_object_limit":268435456,"max_tombstones":64,
                "retained_bytes_limit":1073741824_u64,"minimum_free_bytes":16777216,
                "rename":{"version":1,"whole_folders":[rule("notes",4194304),rule("other",2048)],"whole_files":[]}}
        });
        let fixture = Self {
            path: root.join("config/owner.json"),
            root,
            value,
            vault,
            _temp: temp,
        };
        fixture.write(&fixture.value);
        fixture
    }
    pub fn bytes(&self, bytes: &[u8]) {
        fs::write(&self.path, bytes).unwrap();
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    pub fn write(&self, value: &Value) {
        self.bytes(&serde_json::to_vec(value).unwrap());
    }
    pub fn policy(&self) -> FileOperationPolicy {
        Config::load(&self.path)
            .unwrap()
            .file_operation_policy()
            .unwrap()
            .unwrap()
    }
    pub fn rejects(&self, value: &Value) {
        self.write(value);
        assert!(Config::load(&self.path).is_err());
    }
}
pub(super) fn rule(path: &str, max_bytes: u64) -> Value {
    json!({"path":path,"class":"markdown_text_v1","max_bytes":max_bytes})
}
pub(super) fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
pub(super) fn sync(f: &Fixture, generated: &[&str]) -> Value {
    json!({"version":1,"vault_uuid":f.vault,"link_id":"11111111111111111111111111111111",
        "remote_vault":"22222222222222222222222222222222","scope_epoch":1,
        "whole_folders":[],"whole_files":[{"path":"sync/input.md","markdown":true,
            "attachment_types":[],"max_bytes":1024}],"generated_documents":generated})
}
