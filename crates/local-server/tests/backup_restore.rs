//! 新根恢复重开人侧和接入方两条审计链，以及控制库。
use std::{fs, os::unix::fs::PermissionsExt, time::{Duration, Instant}};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_control_db::ControlDb;
use studiolo_core::checkpoint::{CheckpointId, OwnerWatermark};
use studiolo_local_server::backup::restore::{OwnerPresence, RestoreRequest, restore_into_new_root};
use studiolo_vault_broker::{FsVaultBroker, NewRootRebind};

fn private(path: &std::path::Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn new_root_reopens_broker_control_and_both_ledgers() {
    let root = tempfile::tempdir().unwrap();
    let vault = root.path().join("vault");
    let data = root.path().join("data");
    let broker = FsVaultBroker::open(&vault, &data).unwrap();
    let mut frozen = broker.freeze_checkpoint(Instant::now() + Duration::from_secs(20));
    frozen.verify_audit(|_, _, _| Ok(())).unwrap();
    let restored_vault = root.path().join("new-vault");
    let restored_data = root.path().join("new-data");
    private(&restored_vault);
    private(&restored_data);
    let sealed = frozen.export(&restored_vault, &restored_data).unwrap();
    let OwnerWatermark::Broker { vault_uuid, pair_id, .. } = sealed.watermark else {
        panic!("broker watermark");
    };
    let human = root.path().join("audit");
    let agent = root.path().join("agent-audit");
    let control = root.path().join("control");
    private(&human);
    private(&agent);
    private(&control);
    SqliteAuditLedger::open(&human).unwrap();
    SqliteAuditLedger::open(&agent).unwrap();
    ControlDb::open(control.join("control.sqlite3"), vault_uuid).unwrap();
    let checkpoint = CheckpointId::parse("restore-both-ledgers").unwrap();
    let restored = restore_into_new_root(RestoreRequest {
        broker: NewRootRebind {
            new_vault_root: &restored_vault,
            new_data_root: &restored_data,
            expected_vault_uuid: vault_uuid,
            expected_pair_id: pair_id,
            checkpoint_id: checkpoint.as_str().to_owned(),
            parent_lineage: None,
            resume_interrupted: false,
        },
        control_directory: &control,
        human_audit: &human,
        agent_audit: OwnerPresence::Present(&agent),
        policy: OwnerPresence::Absent,
        agent_store: OwnerPresence::Absent,
    })
    .unwrap();
    assert!(restored.human_audit && restored.agent_audit && restored.control);
    assert!(!restored.policy && !restored.agent_store);
    assert_eq!(restored.broker.vault_uuid, vault_uuid);
    assert!(FsVaultBroker::open_existing(&restored_vault, &restored_data).is_ok());
}
