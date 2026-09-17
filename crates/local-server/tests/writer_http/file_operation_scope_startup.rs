//! 真实账本查询跨等待后，过期的私有批准不得创建 typed 存储。
use super::support::QueryAudit;
use crate::fixture::Fixture;
use serde_json::{Value, json};
use std::{fs, sync::Arc};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_host_config::Config;
use studiolo_local_server::CanonicalWorkbench;
use studiolo_vault_broker::FsVaultBroker;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_drift_inside_startup_query_has_no_typed_storage_side_effect() {
    let f = Fixture::editable().await;
    f.owner.shutdown().await.unwrap();
    let file = f.http.root.join("config/owner.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    config["file_operations"] = json!({
        "version":1,"vault_uuid":f.expected.vault_uuid,"scope_epoch":1,
        "effective_object_limit":4096,"max_tombstones":64,
        "retained_bytes_limit":2u64*1024*1024*1024,"minimum_free_bytes":1024
    });
    fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    let vault = f.http.root.join("vault");
    let roots = Config::load(&file)
        .unwrap()
        .canonical_serving_roots(&vault, &f.http.root.join("web"), &f.data)
        .unwrap();
    let broker = FsVaultBroker::open_existing(&vault, &f.data).unwrap();
    let before = fs::read(f.data.join("writer-state.json")).unwrap();
    let audit = Arc::new(QueryAudit::new(Arc::new(
        SqliteAuditLedger::open_existing(&f.http.audit).unwrap(),
    )));
    let job = {
        let audit = audit.clone();
        tokio::spawn(async move {
            CanonicalWorkbench::prepare_with_audit(broker, Arc::new(roots), audit).await
        })
    };
    let observed = audit.clone();
    tokio::task::spawn_blocking(move || observed.pause.wait())
        .await
        .unwrap();
    config.as_object_mut().unwrap().remove("file_operations");
    fs::write(file, serde_json::to_vec(&config).unwrap()).unwrap();
    audit.pause.release();
    assert!(job.await.unwrap().is_err());
    assert_ne!(f.journal()["file_operations_enabled"], true);
    assert_eq!(fs::read(f.data.join("writer-state.json")).unwrap(), before);
    for path in [
        f.data.join("file-operations-v1"),
        f.data.join("file-operations-v1.preparing"),
        vault.join(".workbench/deleted-objects"),
        vault.join(".workbench/deleted-objects.preparing"),
    ] {
        assert!(!path.exists(), "过期批准不得产生任何 typed 初始化目录");
    }
    assert_eq!(fs::read(vault.join("a.md")).unwrap(), f.expected.content);
    // 失败装配真正归还唯一 owner；没有靠遗留租约把后续打开挡住。
    drop(FsVaultBroker::open_existing(&vault, &f.data).unwrap());
    f.close().await;
}
