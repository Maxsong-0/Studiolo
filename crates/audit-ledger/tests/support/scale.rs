//! 量测，不进门禁（T-220）：几个规模的账本上，单次追加和尾部查询的耗时、每次读的段摘要行数。
//!
//! 要数字时在测试箱上手工跑（发布构建，数字才和生产可比）：
//! `cargo --locked test --release -p studiolo-audit-ledger --lib measure_ -- --ignored --nocapture`。
//! 规模用 `STUDIOLO_MEASURE_SIZES`（逗号分隔的条数，默认 `10000,100000,700000`，700000 是 169 人侧账本 2026-11 的量级）。
//! 打出来的行以 `MEASURE` 开头；最后一行是各规模相对最小规模的追加耗时之比，任一个超过 1.5 就判红
//! （T-220 的完成标准：每次追加的固定成本不随账本总量涨）。
//!
//! 造账本不走 `append`：真追加每条都落盘同步，70 万条要几个小时。这里在一个事务里用和生产同一套编码
//! （`codec::build`、`codec::persistable`、`segment::summarize`）直接写记录、段摘要和 head，写完由
//! `open_existing` 从创世全算一遍段链、核过打开段才开始量，所以量的是一本生产路径认可的账本。
//! 各规模轮流量、每轮每本追加一小批，机器负载的起伏摊到每个规模上，比值才不被某一段时间的抖动带偏。

#![cfg(unix)]

use super::support_tests as support;
use super::{SqliteAuditLedger, codec, segment};
use crate::{
    AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, IntegrityHash,
    SanitizedPayload,
};
use jiff::Timestamp;
use rusqlite::params;
use std::{path::Path, time::Instant};

/// 判红的比值上限：大规模账本上单次追加最多比最小规模慢这么多。
const MAX_RATIO: f64 = 1.5;
/// 每本账本量几轮、每轮追加几条。
const ROUNDS: usize = 8;
const APPENDS_PER_ROUND: usize = 25;
const QUERIES_PER_ROUND: usize = 10;

/// 人侧账本里绝大多数记录的样子：任务对账，正文只有一个 64 位十六进制摘要。
fn event(label: &str, index: u64) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("board:task.reconcile:{label}{index}")).unwrap(),
        AuditPhase::Intent,
        "task.reconcile",
        SanitizedPayload::try_from(serde_json::json!({"request_hash": "0".repeat(64)})).unwrap(),
    )
}

/// 在空账本里一次写下 `records` 条记录、全部封存段摘要和 head。
fn build(root: &Path, records: u64) {
    drop(SqliteAuditLedger::open(root).unwrap());
    let mut connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    let transaction = connection.transaction().unwrap();
    let mut previous = IntegrityHash::genesis();
    let mut segment_previous = segment::genesis();
    let mut segment_first_previous = IntegrityHash::genesis();
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO audit_events(offset,event_id,phase,record_json) VALUES(?1,?2,?3,?4)",
            )
            .unwrap();
        for offset in 1..=records {
            let record =
                codec::build(offset, Timestamp::now(), previous, event("bulk", offset)).unwrap();
            insert
                .execute(params![
                    i64::try_from(offset).unwrap(),
                    record.event().event_id().as_str(),
                    codec::phase(record.event().phase()),
                    codec::persistable(&record).unwrap()
                ])
                .unwrap();
            let index = segment::index_of(offset).unwrap();
            if offset == segment::last_offset(index).unwrap() {
                let summary = segment::summarize(
                    index,
                    segment_first_previous,
                    record.record_hash(),
                    segment_previous,
                )
                .unwrap();
                segment::seal(&transaction, &summary).unwrap();
                segment_previous = summary.segment_hash();
                segment_first_previous = record.record_hash();
            }
            previous = record.record_hash();
        }
    }
    transaction
        .execute(
            "UPDATE audit_meta SET last_offset=?1, last_hash=?2 WHERE id=1",
            params![i64::try_from(records).unwrap(), previous.to_hex()],
        )
        .unwrap();
    transaction.commit().unwrap();
}

struct Measured {
    records: u64,
    ledger: SqliteAuditLedger,
    appended: u64,
    append_seconds: f64,
    append_summaries: u64,
    queries: u64,
    query_seconds: f64,
    query_summaries: u64,
}

impl Measured {
    fn round(&mut self, round: usize) {
        let started = Instant::now();
        for index in 0..APPENDS_PER_ROUND {
            self.ledger
                .append(event(&format!("single{round}-"), index as u64))
                .unwrap();
            self.append_summaries += self.ledger.last_verification_cost().unwrap().summaries();
        }
        self.append_seconds += started.elapsed().as_secs_f64();
        self.appended += APPENDS_PER_ROUND as u64;
        let tail = self.records + self.appended - 1;
        let started = Instant::now();
        for _ in 0..QUERIES_PER_ROUND {
            assert_eq!(
                self.ledger
                    .query(&AuditFilter::default().after_offset(tail))
                    .unwrap()
                    .len(),
                1
            );
            self.query_summaries += self.ledger.last_verification_cost().unwrap().summaries();
        }
        self.query_seconds += started.elapsed().as_secs_f64();
        self.queries += QUERIES_PER_ROUND as u64;
    }

    fn append_ms(&self) -> f64 {
        self.append_seconds * 1000.0 / self.appended as f64
    }
}

#[test]
#[ignore = "量测：造大账本要几十秒到几分钟，只在要数字时手工跑"]
fn measure_append_and_tail_query_cost_across_ledger_sizes() {
    let sizes: Vec<u64> = std::env::var("STUDIOLO_MEASURE_SIZES")
        .unwrap_or_else(|_| "10000,100000,700000".to_owned())
        .split(',')
        .map(|size| size.trim().parse().unwrap())
        .collect();
    let mut directories = Vec::new();
    let mut measured = Vec::new();
    for &records in &sizes {
        let directory = support::private_dir();
        let root = directory.path().canonicalize().unwrap();
        let started = Instant::now();
        build(&root, records);
        let built = started.elapsed().as_secs_f64();
        let started = Instant::now();
        let ledger = SqliteAuditLedger::open_existing(&root).unwrap();
        println!(
            "MEASURE records={records} build_seconds={built:.1} open_ms={:.1} open_summaries={}",
            started.elapsed().as_secs_f64() * 1000.0,
            ledger.last_verification_cost().unwrap().summaries()
        );
        directories.push(directory);
        measured.push(Measured {
            records,
            ledger,
            appended: 0,
            append_seconds: 0.0,
            append_summaries: 0,
            queries: 0,
            query_seconds: 0.0,
            query_summaries: 0,
        });
    }
    for round in 0..ROUNDS {
        for one in &mut measured {
            one.round(round);
        }
    }
    let baseline = measured[0].append_ms();
    let mut worst: f64 = 1.0;
    for one in &measured {
        let ratio = one.append_ms() / baseline;
        worst = worst.max(ratio);
        println!(
            "MEASURE records={} append_ms_avg={:.3} summaries_per_append={:.2} \
             tail_query_ms_avg={:.3} summaries_per_query={:.2} append_ratio_vs_{}={ratio:.2}",
            one.records,
            one.append_ms(),
            one.append_summaries as f64 / one.appended as f64,
            one.query_seconds * 1000.0 / one.queries as f64,
            one.query_summaries as f64 / one.queries as f64,
            measured[0].records,
        );
    }
    println!("MEASURE worst_append_ratio={worst:.2} limit={MAX_RATIO}");
    assert!(
        worst <= MAX_RATIO,
        "单次追加耗时随账本总量涨：最坏比值 {worst:.2} 超过 {MAX_RATIO}"
    );
}
