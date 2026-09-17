use super::support::*;
use crate::sqlite::*;
use crate::*;

#[test]
fn total_length_mismatch_is_rejected_before_bad_payload_is_decoded() {
    let f = Fixture::new();
    drop(f.init());
    let db = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
    db.execute("UPDATE bridge_meta SET payload=X'00',encoded_total=2", [])
        .unwrap();
    drop(db);
    // 若先解析正文，会返回 UnsupportedSchema；必须先命中全量预算校验。
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Budget)
    ));
}
#[test]
fn oversized_schema_sql_is_rejected_without_loading_the_full_string() {
    let f = Fixture::new();
    drop(f.init());
    let db = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
    db.execute_batch("PRAGMA writable_schema=ON;").unwrap();
    db.execute(
        "UPDATE sqlite_schema SET sql=sql||printf('%4096s','') WHERE name='bridge_actions'",
        [],
    )
    .unwrap();
    db.execute_batch("PRAGMA writable_schema=OFF;").unwrap();
    drop(db);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::CorruptStore)
    ));
}
#[test]
fn malformed_role_and_storage_types_are_rejected_before_deserialization() {
    for variant in 0..3 {
        let f = Fixture::new();
        let mut store = f.init();
        let original = input(&store, 1, Presence::Missing, present(9));
        reserve(&mut store, original);
        drop(store);
        let db = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
        db.execute_batch("PRAGMA ignore_check_constraints=ON;")
            .unwrap();
        match variant {
            0 => db
                .execute("UPDATE bridge_plans SET role=printf('%4096s','')", [])
                .unwrap(),
            1 => db
                .execute("UPDATE bridge_plans SET input='not a blob'", [])
                .unwrap(),
            _ => db
                .execute("UPDATE bridge_actions SET payload='not a blob'", [])
                .unwrap(),
        };
        drop(db);
        assert!(matches!(
            BridgeState::open_existing(&f.root, binding()),
            Err(BridgeError::CorruptStore)
        ));
    }
}
