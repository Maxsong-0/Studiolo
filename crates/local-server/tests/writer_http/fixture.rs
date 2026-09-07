use crate::{support, test_support};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc};
use studiolo_core::VaultPath;
use studiolo_host_config::Config;
use studiolo_local_server::CanonicalWorkbench;
use studiolo_vault_broker::{CanonicalRead, FsVaultBroker, ReadOnlyVault};

pub struct Fixture {
    pub http: support::Fixture,
    pub owner: Arc<CanonicalWorkbench>,
    pub data: PathBuf,
    pub expected: CanonicalRead,
}

impl Fixture {
    pub async fn new(watermark: u64, large: bool) -> Self {
        let mut http = support::Fixture::new(true).await;
        let data = http.root.join("writer-data");
        let config_dir = http.root.join("config");
        for path in [&data, &config_dir] {
            fs::create_dir(path).unwrap();
            support::mode(path, 0o700);
        }
        let vault = http.root.join("vault");
        let assets = http.root.join("web");
        if large {
            fs::write(vault.join("exact.md"), vec![b'x'; 1024 * 1024]).unwrap();
            fs::write(vault.join("over.md"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
        }
        drop(FsVaultBroker::open(&vault, &data).unwrap());
        // 合法的历史日志夹具，直接验证超过 JS 安全整数的投影；不伪称执行了海量提交。
        let journal = data.join("writer-state.json");
        let mut state: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        state["committed_through"] = watermark.into();
        state["next_sequence"] = (watermark + 1).into();
        fs::write(&journal, serde_json::to_vec(&state).unwrap()).unwrap();
        let broker = FsVaultBroker::open_existing(&vault, &data).unwrap();
        let expected = broker
            .read_file(&VaultPath::parse("a.md").unwrap(), 1024)
            .unwrap();
        let file = config_dir.join("owner.json");
        fs::write(
            &file,
            serde_json::to_vec(&json!({
                "version": 1, "auth_directory": http.auth, "audit_directory": http.audit,
                "owner": {"issuer": "https://studiolo.cloudflareaccess.com",
                    "audience": "example-audience", "subject": "fictional-person",
                    "email": "maxsong@example.com"},
                "idle_seconds": 600, "absolute_seconds": 1800, "seed_ref": "fixture-v1"
            }))
            .unwrap(),
        )
        .unwrap();
        support::mode(&file, 0o600);
        let roots = Config::load(&file)
            .unwrap()
            .canonical_serving_roots(&vault, &assets, &data)
            .unwrap();
        let owner = CanonicalWorkbench::prepare(broker, Arc::new(roots))
            .await
            .unwrap();
        http.app = studiolo_local_server::router_with_canonical_writer(
            owner.clone(),
            ReadOnlyVault::open(&assets).unwrap(),
            "合成 canonical".into(),
            4319,
            test_support::verifier().await,
            http.manager(),
            None,
        )
        .unwrap();
        Self {
            http,
            owner,
            data,
            expected,
        }
    }

    pub async fn get(&self, uri: &str, cookie: &str, status: u16) -> Value {
        support::json(
            self.http
                .send(self.http.request("GET", uri, Some(cookie), ""), status)
                .await,
        )
        .await
    }

    pub fn journal(&self) -> Value {
        serde_json::from_slice(&fs::read(self.data.join("writer-state.json")).unwrap()).unwrap()
    }

    pub fn set_journal(&self, value: &Value) {
        fs::write(
            self.data.join("writer-state.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }

    pub async fn close(self) {
        self.owner.shutdown().await.unwrap();
    }
}
