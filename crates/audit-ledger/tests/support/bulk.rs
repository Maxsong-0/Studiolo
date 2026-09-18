//! 批量账本夹具：只有需要跨段规模的用例才引入，避免其它测试白付追加成本。

#![cfg(unix)]

use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, path::PathBuf};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditRecord, SanitizedPayload,
    SqliteAuditLedger,
};

/// 造第 `offset` 条记录的事件；event_id 与 offset 一一对应，便于用例直接定位某一条。
pub fn event(offset: u64) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("segmented-{offset}")).unwrap(),
        AuditPhase::Intent,
        "segment.action",
        SanitizedPayload::try_from(json!({"decision": "allow", "序号": offset})).unwrap(),
    )
}

/// 建一个含 `records` 条真实追加记录的账本，关闭后返回目录与其中的全部回执。
///
/// 全部走公开 `append`，因此段摘要也是生产路径写出来的，不是测试自己拼的。
pub fn ledger_with(records: u64) -> (tempfile::TempDir, PathBuf, Vec<AuditRecord>) {
    let directory = crate::support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let receipts = (1..=records)
        .map(|offset| ledger.append(event(offset)).unwrap())
        .collect();
    drop(ledger);
    (directory, root, receipts)
}

/// 把已关闭账本的主库复制到一个新的私有目录；关闭时 WAL 已归并，复制主文件即完整。
pub fn clone_into_private_dir(source_root: &Path) -> (tempfile::TempDir, PathBuf) {
    let directory = crate::support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let target = root.join("audit.sqlite3");
    fs::copy(source_root.join("audit.sqlite3"), &target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    (directory, root)
}

/// 用一条独立连接改写磁盘内容，模拟同权限管理员的篡改；被测账本仍须自行发现。
pub fn tamper(root: &Path, statements: &str) {
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    connection.execute_batch(statements).unwrap();
}
