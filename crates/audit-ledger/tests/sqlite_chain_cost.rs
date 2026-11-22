//! 常规操作的段链验证量与它的安全边界（T-220）。
//!
//! 账本每次读写都要把 head 锚到段链上。以前每次都从创世重算整条段链，读的段摘要行数等于已封存段数，
//! 账本越大每次追加越慢（70 万条约 5470 行）。现在本连接核过的段链留在内存：只要这期间没有别的连接
//! 提交过（`PRAGMA data_version` 没变），每次只重读链尾那一行、核新封存的行。这些用例钉住三件事：
//!
//! 1. 本连接自己的追加、查询、按段续验，读的段摘要行数不随已封存段数涨（以前红：读的行数就是段数）；
//! 2. 别的连接提交过（合法追加也好、篡改也好），下一次操作就从创世全算，打开之后被改的摘要行照样当场失败关闭；
//! 3. 别的连接把账本退回到更早、自身自洽的一版，或退回后再写成同样长的另一条链，本连接也认得出来
//!    （以前认不出：改过之后的账本自己是自洽的）；本实例自己刚封存的那一段被退回掉，同样认得出来。
//!    只读的读窗口（人侧读 agent 账本那条路）也一样。
//!
//! 另钉一条读的顺序：`audit_meta` 标着别的版本、段链又同时坏了，先报版本不对，与 T-220 之前一致。

#![cfg(unix)]

mod support;

#[path = "support/bulk.rs"]
mod bulk;

use std::path::Path;
use studiolo_audit_ledger::{
    AUDIT_SEGMENT_RECORDS, AuditError, AuditFilter, AuditLedger, AuditReadView, AuditRecord,
    SqliteAuditLedger,
};

/// 打开一本有 `sealed` 个封存段、打开段里差两条写满的账本，依次做一遍常规操作，返回每一步读的段摘要行数。
///
/// 中途正好有一次追加把打开段写满并封存，所以「链尾那一行」和「新封存的那一行」两种情形都走到。
fn routine_summary_reads(sealed: u64) -> Vec<(&'static str, u64)> {
    let records = sealed * AUDIT_SEGMENT_RECORDS + AUDIT_SEGMENT_RECORDS - 2;
    let (_source, source_root, _) = bulk::ledger_with(records);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    let reads = |ledger: &SqliteAuditLedger| ledger.last_verification_cost().unwrap().summaries();
    // 打开时仍从创世重算整条段链：这一步的行数就该等于段数，是下面几步的对照组。
    assert_eq!(reads(&ledger), sealed, "打开时没有从创世重算整条段链");
    let mut steps = Vec::new();
    ledger.append(bulk::event(records + 1)).unwrap();
    steps.push(("追加，不封存", reads(&ledger)));
    ledger.append(bulk::event(records + 2)).unwrap();
    steps.push(("追加，写满并封存一段", reads(&ledger)));
    ledger
        .query(&AuditFilter::default().after_offset(records))
        .unwrap();
    steps.push(("查询尾部，读到刚封存的段", reads(&ledger)));
    ledger.append(bulk::event(1)).unwrap();
    steps.push(("幂等重放首段里的一条", reads(&ledger)));
    ledger.verify_segment(0).unwrap();
    steps.push(("按段续验第 0 段", reads(&ledger)));
    // 显式的全量入口照旧从创世全算：行数等于此刻的段数（多了刚封存的那一段）。
    assert_eq!(ledger.segments().unwrap().len() as u64, sealed + 1);
    assert_eq!(reads(&ledger), sealed + 1, "segments() 没有从创世全算");
    assert_eq!(ledger.verify_all().unwrap().summaries(), sealed + 1);
    steps
}

#[test]
fn routine_operations_read_as_many_summaries_on_a_big_ledger_as_on_a_small_one() {
    let small = routine_summary_reads(2);
    let big = routine_summary_reads(9);
    // 按遍数判，不按秒数：段数多了 4.5 倍，常规操作读的段摘要行数一行都不能多。
    assert_eq!(small, big, "常规操作读的段摘要行数随段数涨了");
    for (step, reads) in &big {
        assert!(*reads <= 2, "{step} 读了 {reads} 行段摘要");
    }
}

#[test]
fn a_commit_from_another_connection_makes_the_next_operation_recompute_from_genesis() {
    const SEALED: u64 = 4;
    let (_source, source_root, _) = bulk::ledger_with(SEALED * AUDIT_SEGMENT_RECORDS + 1);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    let reads = |ledger: &SqliteAuditLedger| ledger.last_verification_cost().unwrap().summaries();
    let next = SEALED * AUDIT_SEGMENT_RECORDS + 2;
    ledger.append(bulk::event(next)).unwrap();
    assert!(reads(&ledger) <= 2, "本连接自己的追加还在从创世重算");
    // 另一个实例（另一条连接）追加一条：本连接不知道它动了哪里，下一次操作必须从创世全算。
    let other = SqliteAuditLedger::open_existing(&root).unwrap();
    other.append(bulk::event(next + 1)).unwrap();
    ledger.append(bulk::event(next + 2)).unwrap();
    assert_eq!(
        reads(&ledger),
        SEALED,
        "别的连接提交过，下一次操作没有从创世全算"
    );
    // 全算过一次之后，只要别的连接没再提交，又回到只核链尾。
    ledger.append(bulk::event(next + 3)).unwrap();
    assert!(reads(&ledger) <= 2, "全算之后没有回到只核链尾");
}

#[test]
fn a_summary_changed_after_open_is_caught_by_the_very_next_operation() {
    const SEALED: u64 = 3;
    let (_source, source_root, _) = bulk::ledger_with(SEALED * AUDIT_SEGMENT_RECORDS + 1);
    // 首段、中间段、链尾各种改法；改在本连接打开、已经核过整条段链之后。
    for change in [
        "DELETE FROM audit_segments WHERE segment_index=0",
        "DELETE FROM audit_segments WHERE segment_index=1",
        "DELETE FROM audit_segments WHERE segment_index=2",
        "UPDATE audit_segments SET segment_index=7 WHERE segment_index=1",
        "UPDATE audit_segments SET last_offset=last_offset+1 WHERE segment_index=0",
        "UPDATE audit_segments SET last_hash=(SELECT last_hash FROM audit_segments
            WHERE segment_index=0) WHERE segment_index=1",
        "UPDATE audit_segments SET segment_hash=(SELECT segment_hash FROM audit_segments
            WHERE segment_index=1) WHERE segment_index=2",
        "INSERT INTO audit_segments VALUES(3, 512, '00', '00')",
    ] {
        let (_directory, root) = bulk::clone_into_private_dir(&source_root);
        let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
        ledger
            .append(bulk::event(SEALED * AUDIT_SEGMENT_RECORDS + 2))
            .unwrap();
        bulk::tamper(&root, change);
        assert_eq!(
            ledger
                .append(bulk::event(SEALED * AUDIT_SEGMENT_RECORDS + 3))
                .unwrap_err(),
            AuditError::CorruptStore,
            "打开之后的改动没被下一次追加发现：{change}"
        );
        assert_eq!(
            ledger
                .query(&AuditFilter::default().with_limit(0))
                .unwrap_err(),
            AuditError::CorruptStore,
            "打开之后的改动没被下一次查询发现：{change}"
        );
    }
}

/// 经别的连接把账本退回到只有前 `kept` 条记录的那一版：记录、段摘要、head 三处一起改，退回后的账本自己是自洽的。
fn roll_back_to(root: &Path, receipts: &[AuditRecord], kept: u64) {
    let sealed = kept / AUDIT_SEGMENT_RECORDS;
    let head = receipts[usize::try_from(kept - 1).unwrap()].record_hash();
    bulk::tamper(
        root,
        &format!(
            "DELETE FROM audit_segments WHERE segment_index >= {sealed};
             DELETE FROM audit_events WHERE offset > {kept};
             UPDATE audit_meta SET last_offset={kept}, last_hash='{}' WHERE id=1;",
            head.to_hex()
        ),
    );
}

#[test]
fn rolling_the_ledger_back_to_an_earlier_consistent_version_is_caught_by_the_open_connection() {
    const SEALED: u64 = 3;
    let (_source, source_root, receipts) = bulk::ledger_with(SEALED * AUDIT_SEGMENT_RECORDS + 1);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(ledger.segments().unwrap().len() as u64, SEALED);
    // 退回到只有两个封存段、打开段里一条的那一版。
    let kept = 2 * AUDIT_SEGMENT_RECORDS + 1;
    roll_back_to(&root, &receipts, kept);
    // 正在用的连接核过三个段，库里却只剩两个：段链只能往后长，变短就失败关闭。
    assert_eq!(
        ledger.append(bulk::event(kept + 1)).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger.query(&AuditFilter::default()).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger.verify_segment(0).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(ledger.segments().unwrap_err(), AuditError::CorruptStore);
    // 新打开的连接没见过退回之前的样子，分不出来：这是没有外部可信锚时的已知边界，契约第 7 节写明。
    let fresh = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(fresh.segments().unwrap().len(), 2);
}

#[test]
fn replacing_the_history_with_an_equally_long_consistent_chain_is_caught_by_the_open_connection() {
    const SEALED: u64 = 3;
    let records = SEALED * AUDIT_SEGMENT_RECORDS + 1;
    let (_source, source_root, receipts) = bulk::ledger_with(records);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    assert_eq!(ledger.segments().unwrap().len() as u64, SEALED);
    // 退回到两个封存段，再经别的实例写上另一批事件，长回同样三个封存段：段数没变，第 2 段整段换了内容。
    // 只比段数、不比内容的检查放得过它。
    let kept = 2 * AUDIT_SEGMENT_RECORDS + 1;
    roll_back_to(&root, &receipts, kept);
    let other = SqliteAuditLedger::open_existing(&root).unwrap();
    for offset in kept + 1..=records {
        other.append(bulk::event(offset + 10_000)).unwrap();
    }
    assert_eq!(other.segments().unwrap().len() as u64, SEALED);
    drop(other);
    assert_eq!(
        ledger.append(bulk::event(records + 1)).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger.query(&AuditFilter::default()).unwrap_err(),
        AuditError::CorruptStore
    );
}

#[test]
fn rolling_back_the_segment_this_instance_just_sealed_is_caught_by_its_next_operation() {
    const SEALED: u64 = 3;
    let before_seal = SEALED * AUDIT_SEGMENT_RECORDS + AUDIT_SEGMENT_RECORDS - 1;
    let (_source, source_root, receipts) = bulk::ledger_with(before_seal);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    // 这一次追加写满并封存一段，提交成功，回执已交给调用方。
    ledger.append(bulk::event(before_seal + 1)).unwrap();
    // 别的连接把刚封存的这一段连同最后那条记录抹掉，退回到封存之前自洽的样子。
    roll_back_to(&root, &receipts, before_seal);
    assert_eq!(
        ledger.append(bulk::event(before_seal + 2)).unwrap_err(),
        AuditError::CorruptStore
    );
    assert_eq!(
        ledger
            .query(&AuditFilter::default().with_limit(1))
            .unwrap_err(),
        AuditError::CorruptStore
    );
}

#[test]
fn an_open_read_view_fails_closed_once_another_connection_rewrites_or_rolls_back_the_chain() {
    const SEALED: u64 = 3;
    let (_source, source_root, receipts) = bulk::ledger_with(SEALED * AUDIT_SEGMENT_RECORDS + 1);
    let newest = |view: &AuditReadView| view.scan_page(None, |record| Ok(Some(record.offset())));
    // 读窗口的写者在别的进程，每次读都可能撞上别的连接的提交：先读最新一页，让它核过整条段链。
    let (_rewritten, root) = bulk::clone_into_private_dir(&source_root);
    let view = AuditReadView::open_existing(&root).unwrap();
    assert!(newest(&view).is_ok());
    // 改的是首段：最新一页只读第 1～3 段，读不到它，只有按 `data_version` 判据从创世全算才发现得了。
    bulk::tamper(
        &root,
        "UPDATE audit_segments SET segment_hash=(SELECT segment_hash FROM audit_segments
            WHERE segment_index=1) WHERE segment_index=0",
    );
    assert!(matches!(newest(&view), Err(AuditError::CorruptStore)));
    // 退回到更早、自身自洽的两段：读窗口核过三段，同样失败关闭。
    let (_rolled_back, root) = bulk::clone_into_private_dir(&source_root);
    let view = AuditReadView::open_existing(&root).unwrap();
    assert!(newest(&view).is_ok());
    roll_back_to(&root, &receipts, 2 * AUDIT_SEGMENT_RECORDS + 1);
    assert!(matches!(newest(&view), Err(AuditError::CorruptStore)));
}

#[test]
fn a_wrong_schema_version_is_reported_before_a_broken_segment_chain() {
    const SEALED: u64 = 3;
    let (_source, source_root, _) = bulk::ledger_with(SEALED * AUDIT_SEGMENT_RECORDS + 1);
    let (_directory, root) = bulk::clone_into_private_dir(&source_root);
    let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
    // audit_meta 标着别的版本，段链同时少了一段：先报版本不对（要升级），与 T-220 之前的读取顺序一致。
    bulk::tamper(
        &root,
        "UPDATE audit_meta SET schema_version=99 WHERE id=1;
         DELETE FROM audit_segments WHERE segment_index=1;",
    );
    assert_eq!(
        ledger
            .append(bulk::event(SEALED * AUDIT_SEGMENT_RECORDS + 2))
            .unwrap_err(),
        AuditError::UnsupportedSchema
    );
    drop(ledger);
    assert_eq!(
        SqliteAuditLedger::open_existing(&root).err(),
        Some(AuditError::UnsupportedSchema)
    );
}
