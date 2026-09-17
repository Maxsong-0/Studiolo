use super::super::options::{Command, Options};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_vault_broker::FsVaultBroker;

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub(super) root: PathBuf,
    pub(super) config: PathBuf,
}

impl Fixture {
    pub(super) fn new(audit: bool, writer: bool) -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_path_buf();
        for name in ["config", "vault", "data", "assets", "auth", "audit"] {
            let path = root.join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(root.join("vault/note.md"), "# 私有虚构正文\n").unwrap();
        let config = root.join("config/owner.json");
        fs::write(
            &config,
            serde_json::to_vec(&serde_json::json!({
                "version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
                "owner":{"issuer":"https://fictional.cloudflareaccess.com","audience":"fixture",
                    "subject":"NOT_THE_LOCAL_OPERATOR","email":"fixture@example.com"},
                "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"NOT_A_SECRET_PATH"
            }))
            .unwrap(),
        )
        .unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        if audit {
            drop(SqliteAuditLedger::open(&root.join("audit")).unwrap());
        }
        if writer {
            drop(FsVaultBroker::open(&root.join("vault"), &root.join("data")).unwrap());
        }
        Self {
            _temp: temp,
            root,
            config,
        }
    }

    pub(super) fn options(&self, command: Command) -> Options {
        Options {
            command,
            config: self.config.clone(),
            vault: self.root.join("vault"),
            assets: self.root.join("assets"),
            writer_data: self.root.join("data"),
            request: None,
            maintenance: None,
        }
    }
}
