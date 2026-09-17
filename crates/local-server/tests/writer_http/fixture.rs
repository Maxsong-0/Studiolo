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
        Self::build(watermark, large, false, None, false, None).await
    }

    pub async fn editable() -> Self {
        Self::build(0, false, true, None, false, None).await
    }

    pub async fn with_audit(audit: Arc<dyn studiolo_audit_ledger::AuditLedger>) -> Self {
        Self::build(0, false, true, Some(audit), false, None).await
    }

    pub async fn file_operations(
        audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>>,
    ) -> Self {
        Self::build(0, false, true, audit, true, None).await
    }

    /// 先形成实际源和已存在的目标父目录，再消费同一私有配置批准。
    pub async fn file_rename(
        audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>>,
        content: &[u8],
        sync_generated: bool,
    ) -> Self {
        Self::build(
            0,
            false,
            true,
            audit,
            true,
            Some((content.to_vec(), sync_generated)),
        )
        .await
    }

    async fn build(
        watermark: u64,
        large: bool,
        editable: bool,
        audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>>,
        file_operations: bool,
        rename: Option<(Vec<u8>, bool)>,
    ) -> Self {
        let mut http = support::Fixture::new(true).await;
        let data = http.root.join("writer-data");
        let config_dir = http.root.join("config");
        for path in [&data, &config_dir] {
            fs::create_dir(path).unwrap();
            support::mode(path, 0o700);
        }
        let vault = http.root.join("vault");
        let assets = http.root.join("web");
        if let Some((content, _)) = &rename {
            fs::write(vault.join("a.md"), content).unwrap();
            fs::create_dir(vault.join("files")).unwrap();
            fs::write(
                vault.join("files/generated.md"),
                "只有 Sync 登记，正文没有哨兵。\n",
            )
            .unwrap();
        }
        if large {
            fs::write(vault.join("exact.md"), vec![b'x'; 1024 * 1024]).unwrap();
            fs::write(vault.join("over.md"), vec![b'x'; 1024 * 1024 + 1]).unwrap();
        }
        if editable {
            fs::write(
                vault.join("mixed.md"),
                "正文\n<!-- generated:start -->\n机器区\n<!-- generated:end -->\n",
            )
            .unwrap();
            fs::write(vault.join("CLAUDE.md"), "策略\n").unwrap();
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
            .read_file(
                &VaultPath::parse("a.md").unwrap(),
                if rename.is_some() { 4096 } else { 1024 },
            )
            .unwrap();
        let file = config_dir.join("owner.json");
        let mut config = json!({
            "version": 1, "auth_directory": http.auth, "audit_directory": http.audit,
            "owner": {"issuer": "https://studiolo.cloudflareaccess.com",
                "audience": "example-audience", "subject": "fictional-person",
                "email": "maxsong@example.com"},
            "idle_seconds": 600, "absolute_seconds": 1800, "seed_ref": "fixture-v1"
        });
        if editable {
            config["human_content"] = json!({"version":1,"vault_uuid":expected.vault_uuid,
                "folders":[],"protected":[],"documents":{"a.md":[],
                    "mixed.md":[["<!-- generated:start -->","<!-- generated:end -->"]]}});
        }
        if file_operations {
            config["human_content"]["folders"] = json!(["files"]);
            config["file_operations"] = json!({"version":1,"vault_uuid":expected.vault_uuid,
                "scope_epoch":1,"effective_object_limit":4096,"max_tombstones":64,
                "retained_bytes_limit":2u64*1024*1024*1024,"minimum_free_bytes":1024});
        }
        if let Some((_, sync_generated)) = &rename {
            let rules: Vec<_> = ["a.md", "files/renamed.md", "files/other.md"]
                .iter()
                .map(|path| json!({"path":path,"class":"markdown_text_v1","max_bytes":4096}))
                .collect();
            config["file_operations"]["rename"] = json!({"version":1,
                "whole_folders":[],"whole_files":rules});
            if *sync_generated {
                config["sync_scope"] = json!({"version":1,"vault_uuid":expected.vault_uuid,
                    "link_id":"11111111111111111111111111111111",
                    "remote_vault":"22222222222222222222222222222222","scope_epoch":1,
                    "whole_folders":[],"whole_files":[{"path":"sync/input.md","markdown":true,
                        "attachment_types":[],"max_bytes":1024}],
                    "generated_documents":["files/generated.md"]});
            }
        }
        fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
        support::mode(&file, 0o600);
        let roots = Config::load(&file)
            .unwrap()
            .canonical_serving_roots(&vault, &assets, &data)
            .unwrap();
        let owner = if editable {
            CanonicalWorkbench::prepare_with_audit(
                broker,
                Arc::new(roots),
                audit.unwrap_or_else(|| {
                    Arc::new(
                        studiolo_audit_ledger::SqliteAuditLedger::open_existing(&http.audit)
                            .unwrap(),
                    )
                }),
            )
            .await
            .unwrap()
        } else {
            CanonicalWorkbench::prepare(broker, Arc::new(roots))
                .await
                .unwrap()
        };
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

    pub async fn reopen_file_operations(&mut self) {
        self.reopen_file_operations_with_audit(None).await;
    }

    pub async fn reopen_file_operations_with_audit(
        &mut self,
        audit: Option<Arc<dyn studiolo_audit_ledger::AuditLedger>>,
    ) {
        self.owner.shutdown().await.unwrap();
        let vault = self.http.root.join("vault");
        let assets = self.http.root.join("web");
        let roots = Config::load(&self.http.root.join("config/owner.json"))
            .unwrap()
            .canonical_serving_roots(&vault, &assets, &self.data)
            .unwrap();
        let broker = FsVaultBroker::open_existing(&vault, &self.data).unwrap();
        self.owner = CanonicalWorkbench::prepare_with_audit(
            broker,
            Arc::new(roots),
            audit.unwrap_or_else(|| {
                Arc::new(
                    studiolo_audit_ledger::SqliteAuditLedger::open_existing(&self.http.audit)
                        .unwrap(),
                )
            }),
        )
        .await
        .unwrap();
        self.http.app = studiolo_local_server::router_with_canonical_writer(
            self.owner.clone(),
            ReadOnlyVault::open(&assets).unwrap(),
            "合成 canonical".into(),
            4319,
            test_support::verifier().await,
            self.http.manager(),
            None,
        )
        .unwrap();
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
