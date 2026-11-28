//! 量测，不进门禁（T-251）：人侧账本多大时，变更列表第一页读已读水位（`load`）要多久。
//!
//! `load` 先二分定位账本尾（`tail`），再按 1024 条一页扫最近至多 8192 条，每碰到一条本主体的已读 Outcome
//! 再按事件号查一次成对的 Intent。每次查询都要先把段链锚到 head、走读打开段，再走读命中的段。
//!
//! 要数字时在测试箱全量档手工跑（发布构建，数字才和生产可比）：
//! `cargo --locked test --release -p studiolo-local-server --lib measure_watermark -- --ignored --nocapture`。
//! 规模用 `STUDIOLO_MEASURE_SIZES`（逗号分隔的条数，默认 `10000,100000,300000,700000`；
//! 700000 是 169 人侧账本 2026-11 的量级：701,404 条、689 MB）。
//! 账本按生产入口 `append` 一条条造（每条落盘同步），一本账本逐步长大、每到一个规模量一轮；
//! 填充记录的规范 JSON 约 1 KB，和真账本每条的平均字节数相当，走读一段的哈希量才对得上。
//! 每轮在尾部依次追加 16 对、256 对两个别的主体的已读记录，再追加 4096 对第三个主体的（占满整个尾扫窗口），
//! 分别量命中 0、16、256、4096 对时的 `load`，各量 5 次取中位数。打出来的行以 `MEASURE` 开头，边跑边打，
//! 被测试箱时限掐掉也留得下前面几轮。下一轮估计做不完（`STUDIOLO_MEASURE_BUDGET_SECONDS`，默认 50 分钟）就停。

use super::*;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::SqliteAuditLedger;

/// 填充记录正文里垫的字符数：加上记录头、事件号和两个哈希，规范 JSON 约 1 KB。
const PADDING: usize = 640;
const REPEATS: usize = 5;
const PAIRS: [(char, u64); 3] = [('b', 16), ('c', 256), ('d', 4096)];

fn filler(offset: u64) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("board:task.reconcile:fill{offset}")).unwrap(),
        AuditPhase::Intent,
        "task.reconcile",
        SanitizedPayload::try_from(serde_json::json!({
            "request_hash": "0".repeat(64),
            "note": "x".repeat(PADDING),
        }))
        .unwrap(),
    )
}

fn snapshot(actor: char, vault: VaultUuid, epoch: &str) -> Snapshot {
    Snapshot {
        actor: actor.to_string().repeat(64),
        vault,
        project: None,
        through: 0,
        epoch: epoch.to_owned(),
        observation: 0,
        generation: 0,
    }
}

/// 给 `actor` 追加 `pairs` 对已读记录，水位逐对递增；返回最后一对的水位。
fn mark_seen(ledger: &SqliteAuditLedger, s: &Snapshot, round: usize, pairs: u64) -> u64 {
    // 每轮的水位比上一轮高，上一轮留在窗口里的旧记录压不住这一轮的最大值。
    let base = round as u64 * 10_000;
    for n in 1..=pairs {
        let p = Payload {
            schema: 1,
            actor: s.actor.clone(),
            vault_uuid: s.vault,
            project: None,
            snapshot: VaultUuid::new().to_string(),
            sequence: base + n,
            epoch: s.epoch.clone(),
            observation: n,
            generation: n,
        };
        let id =
            AuditEventId::parse(format!("changes-seen:{}:{}", s.actor, VaultUuid::new())).unwrap();
        append(ledger, id, &p).unwrap_or_else(|_| panic!("追加已读记录"));
    }
    base + pairs
}

/// 量 `REPEATS` 次 `load_inner`，返回中位数毫秒；每次都核结果，免得读失败时落回默认水位、量到一个假快的数。
fn timed(ledger: &SqliteAuditLedger, s: &Snapshot, expected: u64) -> f64 {
    let mut samples: Vec<f64> = (0..REPEATS)
        .map(|_| {
            let started = Instant::now();
            let mark = load_inner(ledger, s).unwrap_or_else(|_| panic!("读已读水位"));
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(mark.sequence, expected, "水位不对，量的不是真扫描");
            elapsed
        })
        .collect();
    samples.sort_by(f64::total_cmp);
    samples[REPEATS / 2]
}

fn fill(ledger: &SqliteAuditLedger, from: u64, to: u64) {
    for offset in from..to {
        ledger.append(filler(offset)).unwrap();
    }
}

#[test]
#[ignore = "量测：按追加造 70 万条账本要十几分钟，只在要数字时手工跑"]
fn measure_watermark_load_across_ledger_sizes() {
    let sizes: Vec<u64> = std::env::var("STUDIOLO_MEASURE_SIZES")
        .unwrap_or_else(|_| "10000,100000,300000,700000".to_owned())
        .split(',')
        .map(|size| size.trim().parse().unwrap())
        .collect();
    let budget = Duration::from_secs(
        std::env::var("STUDIOLO_MEASURE_BUDGET_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3000),
    );
    let directory = tempfile::tempdir().unwrap();
    std::fs::set_permissions(
        directory.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let (vault, epoch) = (VaultUuid::new(), VaultUuid::new().to_string());
    let started = Instant::now();
    let mut filled = 0u64;
    let mut per_append = 0.0f64;
    for (round, &size) in sizes.iter().enumerate() {
        let pending = size.saturating_sub(filled);
        let estimate = per_append * (pending + 2 * 4368) as f64 / 1000.0;
        if started.elapsed().as_secs_f64() + estimate * 1.2 > budget.as_secs_f64() {
            println!(
                "MEASURE stopped records={filled} reason=budget estimate_seconds={estimate:.0}"
            );
            break;
        }
        let chunk = Instant::now();
        fill(&ledger, filled, filled + pending);
        filled += pending;
        if pending > 0 {
            per_append = chunk.elapsed().as_secs_f64() * 1000.0 / pending as f64;
        }
        let db = rusqlite::Connection::open_with_flags(
            root.join("audit.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let (count, bytes): (i64, i64) = db
            .query_row(
                "SELECT count(*), sum(length(record_json)) FROM audit_events",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(db);
        println!(
            "MEASURE records={count} record_bytes_avg={} append_ms_avg={per_append:.3} seconds={:.0}",
            bytes / count.max(1),
            started.elapsed().as_secs_f64()
        );
        let tail_started = Instant::now();
        let end = tail(&ledger).unwrap_or_else(|_| panic!("定位账本尾"));
        println!(
            "MEASURE records={count} tail_ms={:.2} end={end}",
            tail_started.elapsed().as_secs_f64() * 1000.0
        );
        let nobody = snapshot('a', vault, &epoch);
        println!(
            "MEASURE records={count} pairs=0 load_ms_p50={:.2}",
            timed(&ledger, &nobody, 0)
        );
        // 追加完一个主体就量它：4096 对正好占满 8192 条的尾扫窗口，前面主体的记录会被挤出窗口。
        for (actor, pairs) in PAIRS {
            let s = snapshot(actor, vault, &epoch);
            let through = mark_seen(&ledger, &s, round, pairs);
            filled += 2 * pairs;
            println!(
                "MEASURE records={count} pairs={pairs} load_ms_p50={:.2}",
                timed(&ledger, &s, through)
            );
        }
    }
}
