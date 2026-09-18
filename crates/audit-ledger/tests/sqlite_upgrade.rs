//! T-012 v1 账本到 T-051 分段格式的显式升级契约。
//!
//! 夹具做法：先用生产路径建一个 v2 账本，再把它**还原成 v1 形态**——删掉 `audit_segments`、
//! 把 `schema_version` 与 `user_version` 调回 1。这是忠实的 v1，因为两版的 `audit_meta`
//! 与 `audit_events` 建表语句逐字相同、记录编码 `studiolo-audit-json-v1` 也没有改动；
//! 用例开头会把这两点当场断言出来，而不是口头假设。

#![cfg(unix)]

mod support;

#[path = "support/bulk.rs"]
mod bulk;

use std::path::Path;
use studiolo_audit_ledger::{
    AUDIT_SEGMENT_RECORDS, AuditError, AuditFilter, AuditLedger, SqliteAuditLedger,
};

const SEALED_TWICE: u64 = AUDIT_SEGMENT_RECORDS * 2 + 1;

/// T-012 冻结的两张表；测试自带一份独立副本，不从被测代码里取。
const LEGACY_META: &str = "CREATE TABLE audit_meta (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    schema_version INTEGER NOT NULL,
    codec TEXT NOT NULL,
    last_offset INTEGER NOT NULL CHECK(last_offset >= 0),
    last_hash TEXT NOT NULL
) STRICT";
const LEGACY_EVENTS: &str = "CREATE TABLE audit_events (
    offset INTEGER PRIMARY KEY CHECK(offset > 0),
    event_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('intent', 'outcome')),
    record_json TEXT NOT NULL,
    UNIQUE(event_id, phase)
) STRICT";

fn downgrade_to_legacy(root: &Path) {
    bulk::tamper(
        root,
        "DROP TABLE audit_segments;
         UPDATE audit_meta SET schema_version=1;
         PRAGMA user_version=1;",
    );
}

fn schema_objects(root: &Path) -> Vec<(String, Option<String>)> {
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    let mut statement = connection
        .prepare("SELECT name, sql FROM sqlite_schema ORDER BY name")
        .unwrap();
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

fn user_version(root: &Path) -> i64 {
    rusqlite::Connection::open(root.join("audit.sqlite3"))
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn record_bytes(root: &Path) -> Vec<String> {
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    let mut statement = connection
        .prepare("SELECT record_json FROM audit_events ORDER BY offset")
        .unwrap();
    let rows = statement.query_map([], |row| row.get(0)).unwrap();
    rows.collect::<Result<Vec<_>, _>>().unwrap()
}

#[test]
fn a_legacy_ledger_is_refused_until_it_is_explicitly_upgraded_and_never_patched_in_place() {
    let (_source, source_root, receipts) = bulk::ledger_with(SEALED_TWICE);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    downgrade_to_legacy(&root);
    // 夹具忠实性：v1 形态就是三个对象，且两张表的建表语句与 T-012 冻结的逐字相同。
    let legacy = schema_objects(&root);
    assert_eq!(
        legacy,
        vec![
            ("audit_events".to_owned(), Some(LEGACY_EVENTS.to_owned())),
            ("audit_meta".to_owned(), Some(LEGACY_META.to_owned())),
            ("sqlite_autoindex_audit_events_1".to_owned(), None),
        ]
    );
    let before = record_bytes(&root);
    // 两个打开入口都必须明确拒绝，并且不得顺手补表、改版本或重写任何记录。
    for opened in [
        SqliteAuditLedger::open(&root).err(),
        SqliteAuditLedger::open_existing(&root).err(),
    ] {
        assert_eq!(opened, Some(AuditError::UnsupportedSchema));
    }
    assert_eq!(schema_objects(&root), legacy);
    assert_eq!(user_version(&root), 1);
    assert_eq!(record_bytes(&root), before);
    // 显式升级：一次性全链验证，条数与账本规模一致。
    let cost = SqliteAuditLedger::upgrade_existing(&root).unwrap();
    assert_eq!(cost.records(), SEALED_TWICE);
    assert_eq!(cost.summaries(), 0);
    assert_eq!(user_version(&root), 2);
    // 记录正文一个字节都没动，回执与升级前完全一致。
    assert_eq!(record_bytes(&root), before);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap(), receipts);
    // 重建出来的段摘要必须与当初真实追加得到的回执对得上。
    let summaries = ledger.segments().unwrap();
    assert_eq!(summaries.len(), 2);
    for summary in &summaries {
        let tail = &receipts[usize::try_from(summary.last_offset()).unwrap() - 1];
        assert_eq!(summary.last_hash(), tail.record_hash());
        let head = &receipts[usize::try_from(summary.first_offset()).unwrap() - 1];
        assert_eq!(summary.previous_hash(), head.previous_hash());
    }
    assert_eq!(
        summaries[1].previous_segment_hash(),
        summaries[0].segment_hash()
    );
    assert_eq!(ledger.verify_all().unwrap().records(), SEALED_TWICE);
}

#[test]
fn upgrade_refuses_anything_that_is_not_an_intact_legacy_ledger_and_rolls_back() {
    let (_source, source_root, _) = bulk::ledger_with(AUDIT_SEGMENT_RECORDS + 1);
    // 已经是 v2 的库：明确拒绝，而不是把段摘要再建一遍。
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    assert_eq!(
        SqliteAuditLedger::upgrade_existing(&root).err(),
        Some(AuditError::UnsupportedSchema)
    );
    assert_eq!(user_version(&root), 2);
    assert_eq!(schema_objects(&root).len(), 4);
    // 链已经坏掉的 v1 库：升级必须整体回滚，不留下半张段摘要表。
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    downgrade_to_legacy(&root);
    bulk::tamper(
        &root,
        "UPDATE audit_events SET record_json=replace(record_json,'\"allow\"','\"deny\"')
         WHERE offset=2",
    );
    let objects = schema_objects(&root);
    assert_eq!(
        SqliteAuditLedger::upgrade_existing(&root).err(),
        Some(AuditError::CorruptStore)
    );
    assert_eq!(schema_objects(&root), objects);
    assert_eq!(user_version(&root), 1);
    // head 与链尾不符的 v1 库同样拒绝。
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    downgrade_to_legacy(&root);
    bulk::tamper(&root, "UPDATE audit_meta SET last_offset=last_offset-1");
    assert_eq!(
        SqliteAuditLedger::upgrade_existing(&root).err(),
        Some(AuditError::CorruptStore)
    );
    assert_eq!(user_version(&root), 1);
    // 库根本不存在时，升级入口沿用只打开既有库的失败契约，不创建任何文件。
    let empty = support::private_dir();
    let empty_root = empty.path().canonicalize().unwrap();
    assert_eq!(
        SqliteAuditLedger::upgrade_existing(&empty_root).err(),
        Some(AuditError::MissingStore)
    );
    assert_eq!(std::fs::read_dir(&empty_root).unwrap().count(), 0);
}
