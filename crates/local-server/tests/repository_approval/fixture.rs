use crate::ProjectRepository;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};
use studiolo_host_config::Config;

pub struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
}

impl Fixture {
    pub fn new(cargo: bool) -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_owned();
        for directory in ["config", "auth", "audit", "vault", "assets", "repository"] {
            let path = root.join(directory);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir(root.join("repository/src")).unwrap();
        fs::write(
            root.join("repository/Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let config = root.join("config/owner.json");
        let document = json!({"version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture-audience",
                "subject":"fictional-person","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"logical-seed",
            "project_repository":{"version":1,"project":"项目/虚构工坊","directory":root.join("repository"),"cargo_dependencies":cargo}});
        fs::write(&config, serde_json::to_vec(&document).unwrap()).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        Self {
            _temp: temp,
            root,
            config,
        }
    }

    pub fn repository(&self) -> ProjectRepository {
        let approved = Config::load(&self.config)
            .unwrap()
            .repository_serving_roots(
                &self.root.join("vault"),
                &self.root.join("assets"),
                None,
                None,
            )
            .unwrap()
            .unwrap();
        ProjectRepository::open_approved(Arc::new(approved)).unwrap()
    }

    pub fn change_config(&self) {
        let mut document: Value = serde_json::from_slice(&fs::read(&self.config).unwrap()).unwrap();
        document["project_repository"]["project"] = "项目/另一工坊".into();
        fs::write(&self.config, serde_json::to_vec(&document).unwrap()).unwrap();
    }

    pub fn replace_root(&self) {
        fs::rename(
            self.root.join("repository"),
            self.root.join("old-repository"),
        )
        .unwrap();
        fs::create_dir(self.root.join("repository")).unwrap();
    }
}
