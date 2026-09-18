//! 行为契约：复制出来的新目录只有经过 Broker 重绑定才可服务，
//! 且逻辑身份原样保留、lineage 被记录、覆盖到全部绑定位置。

use super::super::{LineageState, MaintenanceCoverage};
use super::{Fixture, note, pair_of};
use crate::{BrokerError, FsVaultBroker, Mutation, VaultWriter};
use std::fs;

#[test]
fn a_copied_pair_only_becomes_servable_after_the_broker_rebinds_the_new_directories() {
    let fixture = Fixture::restored();
    assert!(matches!(
        fixture.open_new(),
        Err(BrokerError::WriterBindingMismatch)
    ));

    let receipt = fixture.rebind().unwrap();

    let broker = fixture.open_new().unwrap();
    assert_eq!(broker.vault_uuid, receipt.vault_uuid);
    assert_eq!(
        broker.write_status().unwrap().committed_through,
        2,
        "提交水位随封存内容一起恢复，不是从零开始"
    );
}

#[test]
fn rebinding_preserves_the_verified_vault_uuid_and_the_logical_pair_id() {
    let fixture = Fixture::restored();
    let sealed = fixture.sealed_binding();

    let receipt = fixture.rebind().unwrap();

    assert_eq!(receipt.vault_uuid, sealed.vault_uuid);
    assert_eq!(receipt.pair_id, sealed.pair_id);
    for binding in [
        fixture.new_vault.join(".workbench/broker-binding.json"),
        fixture.new_data.join("broker-binding.json"),
        fixture.new_data.join("history/meta-v1/binding.json"),
    ] {
        assert_eq!(
            pair_of(&binding),
            sealed.pair_id,
            "{} 必须保留原逻辑 pair_id",
            binding.display()
        );
    }
}

#[test]
fn rebinding_replaces_every_physical_identity_and_records_recovery_lineage() {
    let fixture = Fixture::restored();
    let sealed = fixture.sealed_binding();

    let receipt = fixture.rebind().unwrap();

    assert!(receipt.coverage.covers_required_bindings());
    assert_eq!(receipt.lineage.state, LineageState::Completed);
    assert!(receipt.lineage.is_completed());
    assert_eq!(receipt.lineage.checkpoint_id, "cp-2026-09-17-0001");
    assert_eq!(receipt.lineage.previous_canonical.ino, sealed.canonical.ino);
    assert_ne!(
        receipt.lineage.restored_canonical.unwrap().ino,
        sealed.canonical.ino,
        "新目录的 inode 必须换掉，不能只抄旧身份"
    );
    assert_ne!(
        receipt.lineage.restored_data.unwrap().ino,
        sealed.data.ino,
        "data 根身份同样必须换掉"
    );

    let persisted = super::super::read_restore_lineage(&fixture.new_data)
        .unwrap()
        .unwrap();
    assert_eq!(persisted, receipt.lineage);
}

#[test]
fn the_history_metadata_group_is_rebound_so_the_new_roots_can_serve_history() {
    let fixture = Fixture::restored();
    fixture.rebind().unwrap();

    let broker = fixture.open_new().unwrap();
    let page = broker.list_history(&note(), 8, None).unwrap();
    assert!(
        !page.entries.is_empty(),
        "重绑定后历史目录组必须可用，不能只改主根设备号"
    );
}

#[test]
fn restoring_the_stale_history_binding_after_a_rebind_breaks_history_again() {
    // 反证：如果不重绑定 history/meta-v1，历史就打不开。
    let fixture = Fixture::restored();
    let stale = fs::read(fixture.old_data.join("history/meta-v1/binding.json")).unwrap();
    fixture.rebind().unwrap();
    assert!(fixture.open_new().is_ok());

    fs::write(
        fixture.new_data.join("history/meta-v1/binding.json"),
        &stale,
    )
    .unwrap();
    let broker = fixture.open_new().unwrap();
    assert!(
        broker.list_history(&note(), 8, None).is_err(),
        "旧 history 绑定必须被判为不匹配"
    );
}

#[test]
fn old_idempotency_keys_still_replay_under_the_preserved_pair_id() {
    let fixture = Fixture::restored();
    fixture.rebind().unwrap();

    let mut broker = fixture.open_new().unwrap();
    let replayed = broker
        .mutate(Mutation::create(note(), b"first".to_vec(), "key-one"))
        .unwrap();
    assert_eq!(
        fs::read(fixture.new_vault.join("note.md")).unwrap(),
        b"second".to_vec(),
        "原幂等终态被重放，不会把旧内容再装一次"
    );
    assert!(!replayed.revision.to_hex().is_empty());
}

#[test]
fn an_enabled_maintenance_directory_is_rebound_together_with_the_main_roots() {
    let fixture = Fixture::restored();
    // maintenance-v1 由离线维护流程启用；这里按其固定绑定形状装配一份已启用状态。
    let maintenance = fixture.new_data.join("maintenance-v1");
    fs::create_dir(&maintenance).unwrap();
    let sealed = fixture.sealed_binding();
    let stale_root = serde_json::json!({
        "version": 1,
        "pair_id": sealed.pair_id,
        "vault_uuid": sealed.vault_uuid,
        "root": { "dev": 1_u64, "ino": 1_u64 },
    });
    fs::write(
        maintenance.join("binding.json"),
        serde_json::to_vec(&stale_root).unwrap(),
    )
    .unwrap();
    fs::write(maintenance.join("slot.json"), b"{}").unwrap();

    let receipt = fixture.rebind().unwrap();

    assert_eq!(
        receipt.coverage.maintenance_v1,
        MaintenanceCoverage::Rebound
    );
    let rebound: serde_json::Value =
        serde_json::from_slice(&fs::read(maintenance.join("binding.json")).unwrap()).unwrap();
    assert_ne!(
        rebound["root"]["ino"], 1,
        "maintenance-v1 的根身份必须换成新目录的真实 inode"
    );
    assert_eq!(pair_of(&maintenance.join("binding.json")), sealed.pair_id);
}

#[test]
fn a_vault_without_offline_maintenance_reports_that_it_was_not_enabled() {
    let fixture = Fixture::restored();
    let receipt = fixture.rebind().unwrap();
    assert_eq!(
        receipt.coverage.maintenance_v1,
        MaintenanceCoverage::NotEnabled
    );
}

#[test]
fn the_old_directories_are_left_untouched_by_a_new_directory_restore() {
    let fixture = Fixture::restored();
    let before = fs::read(fixture.old_data.join("broker-binding.json")).unwrap();
    fixture.rebind().unwrap();
    assert_eq!(
        fs::read(fixture.old_data.join("broker-binding.json")).unwrap(),
        before,
        "恢复到新目录，保留旧目录"
    );
    assert!(FsVaultBroker::open_existing(&fixture.old_vault, &fixture.old_data).is_ok());
}
