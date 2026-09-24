//! 契约测试共用夹具：0700 临时目录、镜像视图身份、私有配置里的同步范围、随手写文件。

use serde_json::json;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_host_config::{Config, SyncObservationSelection, SyncScope};
use studiolo_local_server::sync_plane::mirror::MirrorView;
use studiolo_sync_bridge::{Binding, FileVersion};

/// `/tmp` 下一个 0700 的临时目录；路径先规范化，逐段都不是链接。
pub fn private_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

/// 以当前 `(dev, ino)` 取证的视图。
pub fn view(root: &Path) -> MirrorView {
    let meta = fs::metadata(root).unwrap();
    MirrorView::new(root.to_path_buf(), (meta.dev(), meta.ino()))
}

pub fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

pub fn binding() -> Binding {
    Binding {
        link_id: [1; 16],
        canonical_vault: [2; 16],
        remote_vault: [3; 16],
        canonical_root_token: [4; 32],
        mirror_root_token: [5; 32],
        scope_hash: [6; 32],
        scope_epoch: 1,
    }
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

/// 镜像侧冻结版本：revision 就是 sha256。
pub fn version(bytes: &[u8]) -> FileVersion {
    let digest = sha256(bytes);
    FileVersion {
        revision: digest,
        sha256: digest,
        bytes: bytes.len() as u64,
    }
}

/// 在视图根下写一个文件，缺失的父目录顺手建出来。
pub fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let target = root.join(relative);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, bytes).unwrap();
}

/// 私有配置夹具：同步范围为整目录 `笔记`（单文件 ≤ 64 KiB）加精确文件 `附件/手册`（pdf），
/// 生成文档 `笔记/自动汇总.md` 是排除前缀。
pub struct ScopeFixture {
    _temp: tempfile::TempDir,
    path: PathBuf,
}

impl ScopeFixture {
    pub fn new() -> Self {
        let temp = private_dir();
        let root = temp.path().to_path_buf();
        for name in ["config", "auth", "audit"] {
            fs::create_dir(root.join(name)).unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let value = json!({
            "version": 1,
            "auth_directory": root.join("auth"),
            "audit_directory": root.join("audit"),
            "owner": {
                "issuer": "https://fixture.cloudflareaccess.com",
                "audience": "fixture-audience",
                "subject": "fictional-person",
                "email": "owner@example.invalid"
            },
            "idle_seconds": 1800,
            "absolute_seconds": 43200,
            "seed_ref": "logical-seed-id",
            "sync_scope": {
                "version": 1,
                "vault_uuid": VaultUuid::new(),
                "link_id": "11111111111111111111111111111111",
                "remote_vault": "22222222222222222222222222222222",
                "scope_epoch": 1,
                "whole_folders": [
                    {"prefix": "笔记", "markdown": true, "attachment_types": [], "max_bytes": 65536}
                ],
                "whole_files": [
                    {"path": "附件/手册", "markdown": false, "attachment_types": ["pdf"], "max_bytes": 65536}
                ],
                "generated_documents": ["笔记/自动汇总.md"]
            }
        });
        let path = root.join("config/owner.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        Self { _temp: temp, path }
    }

    pub fn scope(&self) -> SyncScope {
        Config::load(&self.path)
            .unwrap()
            .sync_scope()
            .unwrap()
            .unwrap()
    }
}

pub fn selection(scope: &SyncScope) -> SyncObservationSelection {
    scope.observation_selection(scope.binding()).unwrap()
}
