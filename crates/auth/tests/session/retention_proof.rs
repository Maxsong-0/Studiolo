//! 冻结证据按事件取行：缺行是库损坏，未送达是正常等待，锁等真实读取错误仍是存储不可用。

use super::*;
use std::time::Duration;

const EVENT: &str = "store:issue:x";

fn create_outbox(db: &Connection) {
    // 直接取原四表的逐字 DDL，避免测试表与真实格式漂移。
    let (_, ddl) = crate::session::schema_v1::OBJECTS
        .iter()
        .find(|(name, _)| *name == "audit_outbox")
        .unwrap();
    db.execute_batch(ddl.unwrap()).unwrap();
}

fn insert_undelivered(db: &Connection) {
    db.execute(
        "INSERT INTO audit_outbox(sequence,event_id,phase,event_json,event_digest,kind) VALUES(7,?1,'outcome','{}','0','issue')",
        [EVENT],
    )
    .unwrap();
}

#[test]
fn missing_event_row_is_corrupt_store() {
    let db = Connection::open_in_memory().unwrap();
    create_outbox(&db);
    assert_eq!(
        for_event(&db, EVENT, "outcome"),
        Err(SessionError::CorruptStore)
    );
    insert_undelivered(&db);
    assert_eq!(
        for_event(&db, EVENT, "intent"),
        Err(SessionError::CorruptStore)
    );
}

#[test]
fn undelivered_row_waits_and_delivered_row_is_sealed() {
    let db = Connection::open_in_memory().unwrap();
    create_outbox(&db);
    insert_undelivered(&db);
    assert_eq!(for_event(&db, EVENT, "outcome"), Ok(None));
    db.execute(
        "UPDATE audit_outbox SET receipt_json='{}' WHERE sequence=7",
        [],
    )
    .unwrap();
    let proof = for_event(&db, EVENT, "outcome").unwrap().unwrap();
    assert_eq!(proof.sequence, 7);
    assert!(valid_hash(&proof.seal));
}

#[test]
fn locked_read_stays_storage_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("outbox.sqlite");
    let holder = Connection::open(&path).unwrap();
    create_outbox(&holder);
    insert_undelivered(&holder);
    // 回滚日志模式下独占事务阻止其他连接读取，读取方立即得到真实的 SQLITE_BUSY。
    holder.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let reader = Connection::open(&path).unwrap();
    reader.busy_timeout(Duration::ZERO).unwrap();
    assert_eq!(
        for_event(&reader, EVENT, "outcome"),
        Err(SessionError::StorageUnavailable)
    );
    // 放锁后同一读取方立即读到该行，自证上面的失败来自锁而不是缺行或坏表。
    holder.execute_batch("ROLLBACK").unwrap();
    assert_eq!(for_event(&reader, EVENT, "outcome"), Ok(None));
}
