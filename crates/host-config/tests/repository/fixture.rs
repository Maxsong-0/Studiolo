use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use studiolo_host_config::{Config, ConfigError, RepositoryServingRoots};

pub struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
    pub repository: PathBuf,
    pub vault: PathBuf,
    pub assets: PathBuf,
    pub writer: PathBuf,
    pub ingress: PathBuf,
    pub document: Value,
}

pub fn directory(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

impl Fixture {
    pub fn new() -> Self {
        let temporary = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temporary.path().to_path_buf();
        for name in [
            "config",
            "auth",
            "audit",
            "repository",
            "vault",
            "assets",
            "writer",
            "ingress",
        ] {
            directory(&root.join(name));
        }
        let document = json!({"version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture-audience",
                "subject":"fictional-person","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"logical-seed-id",
            "project_repository":{"version":1,"project":"项目/虚构工坊","directory":root.join("repository")}});
        let f = Self {
            config: root.join("config/owner.json"),
            repository: root.join("repository"),
            vault: root.join("vault"),
            assets: root.join("assets"),
            writer: root.join("writer"),
            ingress: root.join("ingress"),
            root,
            document,
            _temp: temporary,
        };
        f.write();
        f
    }

    pub fn write_document(&self, document: &Value) {
        fs::write(&self.config, serde_json::to_vec(document).unwrap()).unwrap();
        fs::set_permissions(&self.config, fs::Permissions::from_mode(0o600)).unwrap();
    }

    pub fn write(&self) {
        self.write_document(&self.document);
    }

    pub fn approve(&self) -> Result<RepositoryServingRoots, ConfigError> {
        Config::load(&self.config)?
            .repository_serving_roots(
                &self.vault,
                &self.assets,
                Some(&self.writer),
                Some(&self.ingress.join("http.sock")),
            )?
            .ok_or(ConfigError)
    }
}
