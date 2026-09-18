//! 漂移契约：恢复用的等价绑定定义必须与上游 Module 实际写出的字节完全一致。
//!
//! `history/meta-v1/binding.json` 与 `maintenance-v1/binding.json` 由各自
//! Module 私有持有，恢复只能用等价定义读写。一旦上游改字段，这里立刻变红。

use super::super::mirror::{HistoryBinding, MaintenanceBinding};
use super::Fixture;
use std::fs;

#[test]
fn the_history_binding_mirror_round_trips_the_bytes_that_history_actually_writes() {
    let fixture = Fixture::restored();
    let bytes = fs::read(fixture.new_data.join("history/meta-v1/binding.json")).unwrap();

    let mirrored: HistoryBinding = serde_json::from_slice(&bytes).unwrap();

    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let round_tripped: serde_json::Value =
        serde_json::from_slice(&serde_json::to_vec(&mirrored).unwrap()).unwrap();
    assert_eq!(
        round_tripped, original,
        "等价定义与真实字节必须逐字段一致，否则重绑定会写坏 history 绑定"
    );
}

#[test]
fn the_maintenance_binding_mirror_matches_its_fixed_shape() {
    let fixture = Fixture::restored();
    let sealed = fixture.sealed_binding();
    let shape = serde_json::json!({
        "version": 1,
        "pair_id": sealed.pair_id,
        "vault_uuid": sealed.vault_uuid,
        "root": { "dev": 7_u64, "ino": 11_u64 },
    });

    let mirrored: MaintenanceBinding = serde_json::from_value(shape.clone()).unwrap();

    let round_tripped: serde_json::Value = serde_json::to_value(&mirrored).unwrap();
    assert_eq!(round_tripped, shape);
}

#[test]
fn an_unknown_field_in_either_binding_is_refused_rather_than_dropped() {
    let mut extra = serde_json::json!({
        "version": 1,
        "pair_id": crate::VaultUuid::new(),
        "vault_uuid": crate::VaultUuid::new(),
        "root": { "dev": 1_u64, "ino": 2_u64 },
        "surprise": true,
    });
    assert!(serde_json::from_value::<MaintenanceBinding>(extra.clone()).is_err());

    extra["requests"] = serde_json::json!({ "dev": 1_u64, "ino": 3_u64 });
    extra["catalogs"] = serde_json::json!({ "dev": 1_u64, "ino": 4_u64 });
    extra["outbox"] = serde_json::json!({ "dev": 1_u64, "ino": 5_u64 });
    assert!(serde_json::from_value::<HistoryBinding>(extra).is_err());
}
