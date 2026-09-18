//! 分段账本的边界、段链、篡改检测与成本契约。
//!
//! 这些用例同时钉住两件事：**分段后常规操作只付「段链 + 读到的段」的代价**，
//! 以及**分段带来的检测时机变化到底是什么**——已封存段内部的改写由 `verify_all`、
//! `verify_segment` 或读到该段的查询发现，而不是由任意一次 append 顺带发现。
//! 后者是明确的取舍，不是遗漏；用例把它写成断言，避免以后被误当成 bug 修成全表重扫。

#![cfg(unix)]

mod support;

#[path = "support/bulk.rs"]
mod bulk;

use studiolo_audit_ledger::{
    AUDIT_SEGMENT_RECORDS, AuditError, AuditEventId, AuditFilter, AuditLedger, IntegrityHash,
    SqliteAuditLedger,
};

/// 两段封存 + 打开段里一条，足以覆盖段边界、段间链接与「中间段」删除。
const SEALED_TWICE: u64 = AUDIT_SEGMENT_RECORDS * 2 + 1;
/// 一段封存 + 打开段里一条，用于只需要「存在已封存段」的用例。
const SEALED_ONCE: u64 = AUDIT_SEGMENT_RECORDS + 1;

#[test]
fn segment_boundaries_are_fixed_and_summaries_link_records_across_them() {
    let (_source, source_root, receipts) = bulk::ledger_with(SEALED_TWICE);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    let summaries = ledger.segments().unwrap();
    assert_eq!(summaries.len(), 2);
    for (position, summary) in summaries.iter().enumerate() {
        let index = position as u64;
        assert_eq!(summary.index(), index);
        assert_eq!(summary.first_offset(), index * AUDIT_SEGMENT_RECORDS + 1);
        assert_eq!(summary.last_offset(), (index + 1) * AUDIT_SEGMENT_RECORDS);
    }
    // 段边界前后各一条：段尾记录哈希进摘要，段首记录的前驱正是上一段段尾。
    let across = ledger
        .query(
            &AuditFilter::default()
                .after_offset(AUDIT_SEGMENT_RECORDS - 1)
                .with_limit(2),
        )
        .unwrap();
    assert_eq!(across[0].offset(), AUDIT_SEGMENT_RECORDS);
    assert_eq!(across[1].offset(), AUDIT_SEGMENT_RECORDS + 1);
    assert_eq!(summaries[0].last_hash(), across[0].record_hash());
    assert_eq!(across[1].previous_hash(), across[0].record_hash());
    // 段链本身：首段接创世，后一段接前一段，既接记录链也接段链。
    assert_eq!(summaries[0].previous_hash(), IntegrityHash::genesis());
    assert_eq!(summaries[1].previous_hash(), summaries[0].last_hash());
    assert_eq!(
        summaries[1].previous_segment_hash(),
        summaries[0].segment_hash()
    );
    assert_ne!(summaries[0].segment_hash(), summaries[1].segment_hash());
    // 已封存段可单独验证；尚未写满的打开段没有摘要，明确返回 None 而不是假装通过。
    assert_eq!(ledger.verify_segment(0).unwrap(), Some(summaries[0]));
    assert_eq!(ledger.verify_segment(1).unwrap(), Some(summaries[1]));
    assert_eq!(ledger.verify_segment(2).unwrap(), None);
    assert_eq!(ledger.verify_segment(u64::MAX).unwrap(), None);
    assert_eq!(
        ledger.query(&AuditFilter::default()).unwrap().len(),
        receipts.len()
    );
}

#[test]
fn restart_append_and_tail_query_stay_bounded_while_full_verification_reads_everything() {
    let (_source, source_root, _) = bulk::ledger_with(SEALED_TWICE);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    // 重启只重算两行段摘要与打开段里的那一条记录，而账本里有 SEALED_TWICE 条。
    let restart = ledger.last_verification_cost().unwrap();
    assert_eq!(restart.summaries(), 2);
    assert_eq!(restart.records(), 1);
    // 尾部查询只走到最后一段：至多「打开段 + 命中段」两段的量。
    assert_eq!(
        ledger
            .query(&AuditFilter::default().after_offset(SEALED_TWICE - 1))
            .unwrap()
            .len(),
        1
    );
    let tail = ledger.last_verification_cost().unwrap();
    assert!(tail.records() <= 2 * AUDIT_SEGMENT_RECORDS, "{tail:?}");
    // 追加同样有界，且不随已封存部分增长。
    let appended = ledger.append(bulk::event(SEALED_TWICE + 1)).unwrap();
    assert_eq!(appended.offset(), SEALED_TWICE + 1);
    let after_append = ledger.last_verification_cost().unwrap();
    assert!(
        after_append.records() <= AUDIT_SEGMENT_RECORDS,
        "{after_append:?}"
    );
    // 对照组：全量验证确实逐条读过全部记录，说明上面的小数字是「按段只验读到的」，
    // 不是「少验了但没人知道」。
    let full = ledger.verify_all().unwrap();
    assert_eq!(full.records(), SEALED_TWICE + 1);
    assert_eq!(full.summaries(), 2);
    assert!(full.records() > 10 * after_append.records(), "{full:?}");
}

#[test]
fn tampering_a_record_inside_a_sealed_segment_is_caught_by_every_path_that_reads_it() {
    let (_source, source_root, _) = bulk::ledger_with(SEALED_ONCE);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let middle = AUDIT_SEGMENT_RECORDS / 2;
    bulk::tamper(
        &root,
        &format!(
            "UPDATE audit_events SET record_json=replace(record_json,'\"allow\"','\"deny\"')
             WHERE offset={middle}"
        ),
    );
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(ledger.verify_all().unwrap_err(), AuditError::CorruptStore);
    assert_eq!(
        ledger.verify_segment(0).unwrap_err(),
        AuditError::CorruptStore
    );
    // 任何会读到该段的查询都必须失败关闭，绝不返回被改过的那条记录。
    for filter in [
        AuditFilter::default(),
        AuditFilter::default().with_limit(1),
        AuditFilter::default()
            .with_event_id(AuditEventId::parse(format!("segmented-{middle}")).unwrap()),
        AuditFilter::default()
            .after_offset(middle - 1)
            .with_limit(1),
    ] {
        assert_eq!(ledger.query(&filter).unwrap_err(), AuditError::CorruptStore);
    }
}

#[test]
fn a_missing_or_rewired_segment_summary_fails_closed_on_every_operation() {
    let (_source, source_root, _) = bulk::ledger_with(SEALED_TWICE);
    for change in [
        "DELETE FROM audit_segments WHERE segment_index=0",
        "DELETE FROM audit_segments WHERE segment_index=1",
        "DELETE FROM audit_segments",
        "UPDATE audit_segments SET segment_index=7 WHERE segment_index=1",
        "UPDATE audit_segments SET last_offset=last_offset+1 WHERE segment_index=0",
        "UPDATE audit_segments SET last_hash=(SELECT last_hash FROM audit_segments
            WHERE segment_index=1) WHERE segment_index=0",
        "UPDATE audit_segments SET segment_hash=(SELECT segment_hash FROM audit_segments
            WHERE segment_index=0) WHERE segment_index=1",
        "UPDATE audit_segments SET last_hash='' WHERE segment_index=1",
        "INSERT INTO audit_segments VALUES(2, 384, '00', '00')",
    ] {
        let (_directory, root) = bulk::clone_into_private_dir(&source_root);
        bulk::tamper(&root, change);
        assert_eq!(
            SqliteAuditLedger::open_existing(&root).err(),
            Some(AuditError::CorruptStore),
            "accepted {change}"
        );
    }
}

#[test]
fn a_summary_that_outlives_its_records_is_caught_when_that_segment_is_read() {
    let (_source, source_root, _) = bulk::ledger_with(SEALED_ONCE);
    // 段尾记录被删：摘要仍宣称该段到 AUDIT_SEGMENT_RECORDS 为止，内容却提前结束。
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    bulk::tamper(
        &root,
        &format!("DELETE FROM audit_events WHERE offset={AUDIT_SEGMENT_RECORDS}"),
    );
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(ledger.verify_all().unwrap_err(), AuditError::CorruptStore);
    assert_eq!(
        ledger.verify_segment(0).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger.query(&AuditFilter::default()).unwrap_err(),
        AuditError::CorruptStore
    );
    // 首条记录被删会让首尾边界立刻对不上，属于每次操作都当场发现的那一类。
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    bulk::tamper(&root, "DELETE FROM audit_events WHERE offset=1");
    assert_eq!(
        SqliteAuditLedger::open_existing(&root).err(),
        Some(AuditError::CorruptStore)
    );
}

#[test]
fn verification_resumes_from_any_segment_and_names_exactly_the_damaged_one() {
    let (_source, source_root, _) = bulk::ledger_with(SEALED_TWICE);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let damaged = AUDIT_SEGMENT_RECORDS + AUDIT_SEGMENT_RECORDS / 2;
    bulk::tamper(
        &root,
        &format!(
            "UPDATE audit_events SET record_json=replace(record_json,'\"allow\"','\"deny\"')
             WHERE offset={damaged}"
        ),
    );
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    let summaries = ledger.segments().unwrap();
    // 逐段续验：可以从任意段号起步，坏的那一段单独暴露，其余段仍各自验证通过。
    assert_eq!(ledger.verify_segment(0).unwrap(), Some(summaries[0]));
    let bounded = ledger.last_verification_cost().unwrap();
    assert!(
        bounded.records() <= 2 * AUDIT_SEGMENT_RECORDS,
        "{bounded:?}"
    );
    assert_eq!(
        ledger.verify_segment(1).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(ledger.verify_all().unwrap_err(), AuditError::CorruptStore);
}

#[test]
fn routine_operations_do_not_reread_sealed_segments_and_full_verification_is_the_backstop() {
    let (_source, source_root, _) = bulk::ledger_with(SEALED_TWICE);
    // 中间那一段的记录被整段删掉，摘要留着：首尾边界仍对得上，只有走读该段才能发现。
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    bulk::tamper(
        &root,
        &format!(
            "DELETE FROM audit_events
             WHERE offset BETWEEN {} AND {}",
            AUDIT_SEGMENT_RECORDS + 1,
            AUDIT_SEGMENT_RECORDS * 2
        ),
    );
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    // 这三条**故意**成功：它们从不读第 1 段，这正是分段换来的成本边界。
    // 契约文档把它写成明确代价；要当场发现就必须重扫整个账本，那正是本片要去掉的东西。
    assert_eq!(
        ledger
            .append(bulk::event(SEALED_TWICE + 1))
            .unwrap()
            .offset(),
        SEALED_TWICE + 1
    );
    assert_eq!(
        ledger
            .query(&AuditFilter::default().after_offset(SEALED_TWICE - 1))
            .unwrap()
            .len(),
        2
    );
    assert!(ledger.segments().unwrap().len() == 2);
    // 兜底：全量验证与该段的单独验证都必须失败关闭。
    assert_eq!(ledger.verify_all().unwrap_err(), AuditError::CorruptStore);
    assert_eq!(
        ledger.verify_segment(1).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger
            .verify_segment(0)
            .unwrap()
            .map(|summary| summary.index()),
        Some(0)
    );
}

#[test]
fn concurrent_instances_cross_a_segment_boundary_and_seal_it_exactly_once() {
    use std::{
        sync::{Arc, Barrier},
        thread,
    };
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    drop(SqliteAuditLedger::open(&root).unwrap());
    // 四个独立实例并发写过同一个段边界；封存与段内最后一条记录在同一事务，
    // 因此无论哪个实例写下最后一条，段摘要都只能出现一次，也不会漏。
    let workers = 4;
    let each = (AUDIT_SEGMENT_RECORDS + 8) / workers;
    let barrier = Arc::new(Barrier::new(usize::try_from(workers).unwrap()));
    let handles: Vec<_> = (0..workers)
        .map(|worker| {
            let barrier = Arc::clone(&barrier);
            let ledger = SqliteAuditLedger::open(&root).unwrap();
            thread::spawn(move || {
                barrier.wait();
                for index in 0..each {
                    ledger
                        .append(bulk::event(worker * each + index + 1))
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let reopened = SqliteAuditLedger::open_existing(&root).unwrap();
    let total = workers * each;
    assert!(total > AUDIT_SEGMENT_RECORDS);
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap().len(),
        usize::try_from(total).unwrap()
    );
    let summaries = reopened.segments().unwrap();
    assert_eq!(
        summaries.len(),
        usize::try_from(total / AUDIT_SEGMENT_RECORDS).unwrap()
    );
    assert_eq!(summaries[0].last_offset(), AUDIT_SEGMENT_RECORDS);
    assert_eq!(reopened.verify_all().unwrap().records(), total);
}
