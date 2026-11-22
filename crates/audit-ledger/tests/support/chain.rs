//! 段链缓存在 `data_version` 看不见的改动面前的表现（T-220）。
//!
//! 别的连接提交过，`data_version` 就变，下一次操作从创世全算（集成用例 `sqlite_chain_cost.rs`）。
//! 看不见的改动（不经 SQLite 直接改文件字节）在用例里造不出来，这里用被测连接**自己**改库来模拟：
//! 本连接自己的提交不改变 `data_version`，下一次常规操作只核链尾。钉住两件事：链尾那一行被删当场失败关闭；
//! 中间一行被改，常规追加照常成功（契约第 8 节写明的检测时机变化），但读到那一段、`segments()`、
//! `verify_all()` 和重新打开都失败关闭。

#![cfg(unix)]

use super::support_tests as support;
use super::{SqliteAuditLedger, segment::SEGMENT_RECORDS};
use crate::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
};

const SEALED: u64 = 3;

fn event(offset: u64) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("chain-{offset}")).unwrap(),
        AuditPhase::Intent,
        "chain.action",
        SanitizedPayload::try_from(serde_json::json!({"序号": offset})).unwrap(),
    )
}

/// 建一本三个封存段、打开段里一条的账本，返回目录与打开它的实例；先做一次追加，让缓存进入「只核链尾」。
fn ledger() -> (tempfile::TempDir, std::path::PathBuf, SqliteAuditLedger) {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    for offset in 1..=SEALED * SEGMENT_RECORDS + 1 {
        ledger.append(event(offset)).unwrap();
    }
    assert!(ledger.last_verification_cost().unwrap().summaries() <= 2);
    (directory, root, ledger)
}

/// 用被测连接自己执行改动：`data_version` 不变，模拟别的连接看不见的那类改动。
fn rewrite_through_own_connection(ledger: &SqliteAuditLedger, sql: &str) {
    let session = ledger.connection.lock().unwrap();
    assert_eq!(session.connection.execute(sql, []).unwrap(), 1);
}

#[test]
fn an_unseen_change_to_the_chain_tip_fails_the_very_next_operation() {
    let tip = SEALED - 1;
    for change in [
        format!("DELETE FROM audit_segments WHERE segment_index={tip}"),
        format!(
            "UPDATE audit_segments SET segment_hash=(SELECT segment_hash FROM audit_segments
                WHERE segment_index=0) WHERE segment_index={tip}"
        ),
        format!("UPDATE audit_segments SET last_offset=last_offset-1 WHERE segment_index={tip}"),
    ] {
        let (_directory, _root, ledger) = ledger();
        rewrite_through_own_connection(&ledger, &change);
        assert_eq!(
            ledger
                .append(event(SEALED * SEGMENT_RECORDS + 2))
                .unwrap_err(),
            AuditError::CorruptStore,
            "链尾被改，下一次追加没发现：{change}"
        );
    }
}

#[test]
fn an_unseen_rewrite_of_a_middle_summary_is_caught_when_that_segment_is_read() {
    let (_directory, root, ledger) = ledger();
    rewrite_through_own_connection(
        &ledger,
        "UPDATE audit_segments SET segment_hash=(SELECT segment_hash FROM audit_segments
            WHERE segment_index=1) WHERE segment_index=0",
    );
    // 常规追加只核链尾，不读第 0 段：照常成功，这是契约写明的代价。
    ledger.append(event(SEALED * SEGMENT_RECORDS + 2)).unwrap();
    // 读到第 0 段的每条路都重读它的摘要行，当场失败关闭。
    for filter in [
        AuditFilter::default().with_limit(1),
        AuditFilter::default()
            .with_event_id(AuditEventId::parse("chain-1").unwrap())
            .with_phase(AuditPhase::Intent),
    ] {
        assert_eq!(ledger.query(&filter).unwrap_err(), AuditError::CorruptStore);
    }
    assert_eq!(
        ledger.append(event(1)).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger.verify_segment(0).unwrap_err(),
        AuditError::CorruptStore
    );
    // 第 1 段没被改：照常读得到。
    assert!(ledger.verify_segment(1).unwrap().is_some());
    // 显式入口从创世全算，重新打开也全算。
    assert_eq!(ledger.segments().unwrap_err(), AuditError::CorruptStore);
    assert_eq!(ledger.verify_all().unwrap_err(), AuditError::CorruptStore);
    drop(ledger);
    assert_eq!(
        SqliteAuditLedger::open_existing(&root).err(),
        Some(AuditError::CorruptStore)
    );
}
