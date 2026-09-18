//! 候选行查询的执行计划证据：断言的是被测连接上 `EXPLAIN QUERY PLAN` 的真实输出。

#![cfg(unix)]

use super::support_tests as support;

use super::{SqliteAuditLedger, query};
use crate::{AuditEventId, AuditFilter, AuditPhase};
use studiolo_core::{TaskId, ToolId};

#[test]
fn every_filter_shape_plans_onto_an_index_instead_of_a_table_scan() {
    let directory = support::private_dir();
    let ledger = SqliteAuditLedger::open(&directory.path().canonicalize().unwrap()).unwrap();
    let session = ledger.connection.lock().unwrap();
    let event_id = AuditEventId::parse("planned").unwrap();
    // SQLite 用 SCAN 表示无界整表扫描；出现它就说明候选集又回到了 O(账本总量)。
    // event_id 等值最多命中 intent/outcome 两行，排序用的临时表规模恒定，因此只在
    // 结果集无界的计划里禁止 TEMP B-TREE。
    for (label, filter, required, forbidden) in [
        (
            "无过滤器仍从 rowid 主键的 offset > 0 起步",
            AuditFilter::default(),
            "USING INTEGER PRIMARY KEY",
            ["SCAN audit_events", "TEMP B-TREE"].as_slice(),
        ),
        (
            "after_offset 直接落在 rowid 范围上",
            AuditFilter::default().after_offset(100_000),
            "USING INTEGER PRIMARY KEY",
            ["SCAN audit_events", "TEMP B-TREE"].as_slice(),
        ),
        (
            "event_id 命中唯一索引首列",
            AuditFilter::default().with_event_id(event_id.clone()),
            "sqlite_autoindex_audit_events_1",
            ["SCAN audit_events"].as_slice(),
        ),
        (
            "event_id 与 phase 命中完整唯一键",
            AuditFilter::default()
                .with_event_id(event_id.clone())
                .with_phase(AuditPhase::Intent),
            "sqlite_autoindex_audit_events_1",
            ["SCAN audit_events"].as_slice(),
        ),
        (
            "phase 不是索引首列，退回有界的 rowid 范围而不是整表扫描",
            AuditFilter::default().with_phase(AuditPhase::Outcome),
            "USING INTEGER PRIMARY KEY",
            ["SCAN audit_events", "TEMP B-TREE"].as_slice(),
        ),
        (
            "正文里的 subject 过滤没有索引，仍只走有界 rowid 范围",
            AuditFilter::default()
                .with_task(TaskId::new())
                .with_tool(ToolId::from_static("web.fetch"))
                .with_action("session.establish"),
            "USING INTEGER PRIMARY KEY",
            ["SCAN audit_events", "TEMP B-TREE"].as_slice(),
        ),
    ] {
        let (sql, plan) = query::explain(&session.connection, &filter).unwrap();
        assert!(plan.contains(required), "{label}: {sql} -> {plan}");
        for pattern in forbidden {
            assert!(!plan.contains(pattern), "{label}: {sql} -> {plan}");
        }
    }
}

#[test]
fn only_fully_pushed_down_filters_may_carry_the_limit_into_sqlite() {
    let directory = support::private_dir();
    let ledger = SqliteAuditLedger::open(&directory.path().canonicalize().unwrap()).unwrap();
    let session = ledger.connection.lock().unwrap();
    let (indexed, _) = query::explain(
        &session.connection,
        &AuditFilter::default()
            .with_event_id(AuditEventId::parse("planned").unwrap())
            .with_limit(1),
    )
    .unwrap();
    assert!(indexed.ends_with("ORDER BY offset LIMIT ?"), "{indexed}");
    // action 只存在于规范 JSON 正文里；此时候选集必须完整，limit 不能下推到 SQLite。
    let (unindexed, _) = query::explain(
        &session.connection,
        &AuditFilter::default()
            .with_action("session.establish")
            .with_limit(1),
    )
    .unwrap();
    assert!(unindexed.ends_with("ORDER BY offset"), "{unindexed}");
}
