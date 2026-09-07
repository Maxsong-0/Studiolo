//! 固定五根、既有 Broker 布局与真实监视；仅虚构私有目录。

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use studiolo_host_config::Config;
use studiolo_vault_broker::FsVaultBroker;

use super::TIMEOUT;
use crate::CanonicalWorkbench;

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for name in ["config", "vault", "data", "assets", "auth", "audit"] {
            let path = root.join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(root.join("vault/note.md"), "# 原始虚构内容\n").unwrap();
        let config = root.join("config/owner.json");
        fs::write(&config, serde_json::to_vec(&serde_json::json!({
            "version":1, "auth_directory":root.join("auth"), "audit_directory":root.join("audit"),
            "owner":{"issuer":"https://studiolo.cloudflareaccess.com", "audience":"example-audience",
            "subject":"fictional-person", "email":"maxsong@example.com"},
            "idle_seconds":1800, "absolute_seconds":43200, "seed_ref":"synthetic-seed"
        })).unwrap()).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        drop(FsVaultBroker::open(&root.join("vault"), &root.join("data")).unwrap());
        Self {
            _temp: temp,
            root,
            config,
        }
    }

    pub async fn start(&self, os_events: bool) -> Arc<CanonicalWorkbench> {
        let config = Config::load(&self.config).unwrap();
        let roots = Arc::new(
            config
                .canonical_serving_roots(
                    &self.root.join("vault"),
                    &self.root.join("assets"),
                    &self.root.join("data"),
                )
                .unwrap(),
        );
        let broker =
            FsVaultBroker::open_existing(&self.root.join("vault"), &self.root.join("data"))
                .unwrap();
        let workbench = CanonicalWorkbench::prepare(broker, roots).await.unwrap();
        idle(&workbench).await;
        workbench
            .core
            .probe
            .os_events
            .store(os_events, Ordering::Release);
        workbench
    }

    pub fn available(&self) {
        drop(
            FsVaultBroker::open_existing(&self.root.join("vault"), &self.root.join("data"))
                .unwrap(),
        );
    }

    pub fn journal(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.root.join("data/writer-state.json")).unwrap())
            .unwrap()
    }

    pub fn replace_journal(&self, value: &serde_json::Value) {
        let path = self.root.join("data/writer-state.json");
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }
}

pub(super) async fn idle(workbench: &CanonicalWorkbench) {
    let mut changed = workbench.core.completion.subscribe();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let _ = changed.borrow_and_update();
            if !workbench.core.control.lock().unwrap().running {
                return;
            }
            changed.changed().await.unwrap();
        }
    })
    .await
    .expect("真实协调器作业未结束");
}

pub(super) async fn entered(receiver: tokio::sync::oneshot::Receiver<()>) {
    tokio::time::timeout(TIMEOUT, receiver)
        .await
        .expect("作业没有到达同步点")
        .unwrap();
}
