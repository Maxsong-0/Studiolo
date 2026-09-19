//! `admin audit-upgrade` 契约：把线上那种 T-012 v1 账本升成分段格式，且只升这一种。
//!
//! 为什么有这个测试：T-051 把账本磁盘格式升到 v2 之后，`open`／`open_existing` 一律拒绝 v1，
//! 而升级只有库 API、没有任何生产程序调用它。2026-09-18 真上线时 4401 阶段实例因此起不来
//! （[[BUG-014]]：线上账本是 v1、356 条记录）。此处用**真实的 v1 账本**跑完整路径：
//! 先证明新二进制确实打不开它，升级后能打开、记录一条不少、再升一次必须拒绝。
//!
//! 夹具做法与 `studiolo-audit-ledger` 的升级契约一致：先用生产路径建 v2，再还原成 v1 形态
//! （删 `audit_segments`、`schema_version` 与 `user_version` 调回 1）。两版的 `audit_meta`
//! 与 `audit_events` 建表语句逐字相同、记录编码未变，所以这是忠实的 v1。

#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use studiolo_agent_server::{admin, options::AdminCommand};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
    SqliteAuditLedger,
};
use tempfile::TempDir;

fn private_dir(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn event(index: u64) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("t082-fixture-{index}")).unwrap(),
        AuditPhase::Intent,
        "agent.fixture",
        SanitizedPayload::try_from(serde_json::json!({ "index": index })).unwrap(),
    )
}

/// 把 v2 账本还原成 T-012 的 v1 形态；只动 schema 与版本，不碰任何记录。
fn downgrade_to_legacy(audit: &Path) {
    let connection = rusqlite::Connection::open(audit.join("audit.sqlite3")).unwrap();
    connection
        .execute_batch(
            "DROP TABLE audit_segments;
             UPDATE audit_meta SET schema_version=1;
             PRAGMA user_version=1;",
        )
        .unwrap();
    drop(connection);
    // 账本要求目录 0700、文件 0600；直连造夹具时 WAL 侧文件按 umask 创建，这里补回来，
    // 否则后面的失败会是 UnsafeStorage 而不是我们要验的「v1 被拒绝」。
    for entry in fs::read_dir(audit).unwrap() {
        let path = entry.unwrap().path();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn record_ids(audit: &Path) -> Vec<String> {
    SqliteAuditLedger::open_existing(audit)
        .unwrap()
        .query(&AuditFilter::default())
        .unwrap()
        .iter()
        .map(|record| record.event().event_id().as_str().to_owned())
        .collect()
}

#[test]
fn a_real_v1_ledger_is_refused_until_admin_upgrades_it_and_upgrading_twice_is_refused() {
    // 账本按设计拒绝含符号链接的路径；macOS 的 /var/folders 正是那样，必须先规范化。
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let data = temp.path().join("agent-data");
    let audit = data.join("audit");
    private_dir(&data);
    private_dir(&audit);

    let ledger = SqliteAuditLedger::open(&audit).unwrap();
    for index in 0..12 {
        ledger.append(event(index)).unwrap();
    }
    let before = record_ids(&audit);
    assert_eq!(before.len(), 12);
    drop(ledger);

    downgrade_to_legacy(&audit);
    // 前提自证：新二进制确实打不开这种账本——线上 2026-09-18 撞到的就是这一步。
    assert!(
        SqliteAuditLedger::open_existing(&audit).is_err(),
        "v1 账本必须被拒绝，否则本用例的前提不成立"
    );

    admin::run(AdminCommand::AuditUpgrade { data: data.clone() }).unwrap();

    // 升级后能打开，且记录逐条一致：升级只加派生的段摘要，不改写任何既有记录。
    assert_eq!(record_ids(&audit), before);

    // 同一账本再升一次必须拒绝：升级是一次性、单向的，不得对 v2 重复施加。
    let again = admin::run(AdminCommand::AuditUpgrade { data: data.clone() });
    assert!(again.is_err(), "对已是 v2 的账本重复升级必须拒绝");
    assert_eq!(record_ids(&audit), before, "被拒绝的升级不得改动记录");
}

#[test]
fn upgrade_refuses_a_missing_or_unreadable_ledger_without_creating_one() {
    // 账本按设计拒绝含符号链接的路径；macOS 的 /var/folders 正是那样，必须先规范化。
    let temp = TempDir::new_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    let data = temp.path().join("agent-data");
    private_dir(&data);
    private_dir(&data.join("audit"));

    // 空目录里没有账本：拒绝，且不得顺手建一个（那会把「忘了指对目录」变成「静默新建空账本」）。
    assert!(admin::run(AdminCommand::AuditUpgrade { data: data.clone() }).is_err());
    assert!(!data.join("audit/audit.sqlite3").exists());
}
