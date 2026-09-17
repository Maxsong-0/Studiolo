//! 启用发生在实际Broker绑定和真实账本可读性核验之后；不靠配置UUID自证身份。
use crate::fixture::Fixture;
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord};
use studiolo_host_config::Config;
use studiolo_local_server::CanonicalWorkbench;
use studiolo_vault_broker::FsVaultBroker;

struct Unavailable;
impl AuditLedger for Unavailable {
    fn append(&self, _: AuditEvent) -> Result<AuditRecord, AuditError> {
        Err(AuditError::Unavailable)
    }
    fn query(&self, _: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        Err(AuditError::Unavailable)
    }
}

#[tokio::test]
async fn foreign_config_binding_or_unavailable_audit_never_enable_file_operation_storage() {
    for wrong_vault in [false, true] {
        let f = Fixture::editable().await;
        f.owner.shutdown().await.unwrap();
        let file = f.http.root.join("config/owner.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        let vault = if wrong_vault {
            studiolo_core::VaultUuid::new()
        } else {
            f.expected.vault_uuid
        };
        config["human_content"]["vault_uuid"] = json!(vault);
        config["file_operations"] = json!({"version":1,"vault_uuid":vault,"scope_epoch":1,
            "effective_object_limit":4096,"max_tombstones":64,"retained_bytes_limit":2u64*1024*1024*1024,"minimum_free_bytes":1024});
        fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
        let vault_root = f.http.root.join("vault");
        let assets = f.http.root.join("web");
        let roots = Config::load(&file)
            .unwrap()
            .canonical_serving_roots(&vault_root, &assets, &f.data)
            .unwrap();
        let broker = FsVaultBroker::open_existing(&vault_root, &f.data).unwrap();
        let audit: Arc<dyn AuditLedger> = if wrong_vault {
            Arc::new(
                studiolo_audit_ledger::SqliteAuditLedger::open_existing(&f.http.audit).unwrap(),
            )
        } else {
            Arc::new(Unavailable)
        };
        assert!(
            CanonicalWorkbench::prepare_with_audit(broker, Arc::new(roots), audit)
                .await
                .is_err()
        );
        assert_ne!(f.journal()["file_operations_enabled"], true);
        assert!(!vault_root.join(".workbench/deleted-objects").exists());
        assert_eq!(
            fs::read(vault_root.join("a.md")).unwrap(),
            f.expected.content
        );
        f.close().await;
    }
}
