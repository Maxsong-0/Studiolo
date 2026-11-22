//! 备份队列自己把账本读到末尾，不靠外面的请求一页一页推（BUG-042）。
//!
//! 人侧每次启动，队列的进度都从零开始；账本有 70 万条时，以前只有 worker、定时器、健康页的请求推它往前读，
//! 一条请求最多读 5 秒，头十几分钟备份 socket 一条都答不上来。
use super::{producer::Stage, *};
use std::sync::atomic::AtomicBool;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_backup::worker::audit::{QueueAnswer, QueueCommand};

/// 用例给后台追账本用的页大小；账本要放满两页多，才分得清「读了一页」和「读完了」。
/// 真跑时的一页（`catch_up.rs` 的 `CATCH_UP_PAGE`）大得多，用例里的账本装不满它。
const PAGE: usize = 512;

/// 往人侧账本里追加 `count` 条和备份无关的记录，返回末条的偏移。人侧跑久了，账本里绝大多数都是这种。
fn fill(stage: &Stage, label: &str, count: usize) -> u64 {
    let mut last = 0;
    for index in 0..count {
        let event = AuditEvent::new(
            AuditEventId::parse(format!("board:task.reconcile:{label}{index}")).unwrap(),
            AuditPhase::Intent,
            "task.reconcile",
            SanitizedPayload::try_from(serde_json::json!({"request_hash": "0".repeat(64)}))
                .unwrap(),
        );
        last = stage.assembled.human.append(event).unwrap().offset();
    }
    last
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catch_up_reads_the_whole_ledger_without_any_request() {
    let stage = Stage::new(true).await;
    let last = fill(&stage, "idle", 2 * PAGE + 7);
    assert_eq!(
        stage.producer.scanned_offset(),
        0,
        "没人发请求，进度不会自己动"
    );
    // 小页：读满一页接着读下一页，最后一页不满就收手。
    stage.producer.catch_up_by(&AtomicBool::new(false), PAGE);
    assert_eq!(stage.producer.scanned_offset(), last);
    // 读完之后请求照旧答得上来，结论和没读过这些无关记录时一样。
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    // 真跑时的大页一页就装下整本账本，读到的位置一样。
    let restarted = stage.restarted();
    restarted.catch_up(&AtomicBool::new(false));
    assert_eq!(restarted.scanned_offset(), last);
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catch_up_reads_nothing_once_it_is_told_to_stop() {
    let stage = Stage::new(true).await;
    fill(&stage, "stopped", PAGE + 3);
    // 关停先置停止标志：已经置上的话，一页都不再读。
    stage.producer.catch_up_by(&AtomicBool::new(true), PAGE);
    assert_eq!(stage.producer.scanned_offset(), 0);
    stage.producer.catch_up(&AtomicBool::new(true));
    assert_eq!(stage.producer.scanned_offset(), 0);
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restarted_producer_catches_up_to_the_same_answers_as_before_the_restart() {
    let stage = Stage::new(true).await;
    fill(&stage, "before", PAGE + 5);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    let last = fill(&stage, "after", PAGE + 5);
    let QueueAnswer::Pending(before) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };

    // 重启：同一本账本，队列进度从零开始，自己读完。
    let restarted = stage.restarted();
    restarted.catch_up(&AtomicBool::new(false));
    assert_eq!(restarted.scanned_offset(), last);
    let ask = |command| {
        let request = studiolo_backup::worker::audit::BackupQueueRequest::new(
            crate::backup::producer::TARGET,
            command,
        );
        request
            .decode_reply(&restarted.dispatch(&request.encode().unwrap()).unwrap())
            .unwrap()
    };
    // 夹在无关记录中间的那次封存只算一次：待办还是同一条，离上一次太近、不会再发新序号。
    assert_eq!(ask(QueueCommand::NextPending), QueueAnswer::Pending(before));
    assert_eq!(ask(QueueCommand::Schedule), QueueAnswer::Outstanding);
    restarted.settle();
    assert_eq!(stage.outcomes(2), 0);
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 监听起来之后一条请求都不发：进度只能是它自己推的。
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_listener_catches_up_on_its_own_and_stops_reading_on_shutdown() {
    let stage = Stage::new(true).await;
    let last = fill(&stage, "listening", 2 * PAGE + 7);
    let run = target(&stage.assembled.fixture, "backup-run");
    fs::set_permissions(&run, fs::Permissions::from_mode(0o750)).unwrap();
    let service =
        crate::backup::BackupService::start(stage.producer.clone(), &run.join("producer.sock"))
            .unwrap();
    let limit = Instant::now() + Duration::from_secs(30);
    while stage.producer.scanned_offset() != last {
        assert!(
            Instant::now() < limit,
            "监听起来之后没有自己把账本读完：读到 {}，末条 {last}",
            stage.producer.scanned_offset()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    service.shutdown().await;
    // 关停可以重复调用；追账本的线程已经收走，不会再动进度。
    service.shutdown().await;
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 量测，不进门禁（造一本大账本要很久）：要数字时手工跑
/// `STUDIOLO_MEASURE_LEDGER=700000 cargo test --release -p studiolo-local-server --lib measure_ -- --ignored --nocapture`。
/// 打出来的行以 `MEASURE` 开头。账本每满 10 万条量一轮：首尾查询改前改后各一次的耗时、单次追加、
/// 重启后把账本追完要多久；到头再做一次封存。分几个点量而不是只量终点：造账本每条追加都落盘同步，
/// 很慢，测试箱单任务又有时限。下一轮估计做不完（`STUDIOLO_MEASURE_BUDGET_SECONDS`，默认 48 分钟）就不再往下造，
/// 直接去量封存，前面几轮的数照样留下。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "量测：造一本大账本要很久，只在要数字时手工跑"]
async fn measure_catch_up_append_and_seal_over_a_large_ledger() {
    const STEP: usize = 100_000;
    let count: usize = std::env::var("STUDIOLO_MEASURE_LEDGER")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(700_000);
    let budget = Duration::from_secs(
        std::env::var("STUDIOLO_MEASURE_BUDGET_SECONDS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(2880),
    );
    let stage = Stage::new(true).await;
    let started = Instant::now();
    let mut last = 0;
    let mut previous = Duration::ZERO;
    for (round, start) in (0..count).step_by(STEP).enumerate() {
        // 每一轮都比上一轮慢：按上一轮用时的 1.3 倍估，装不进预算就停。
        if started.elapsed() + previous.mul_f64(1.3) > budget {
            println!("MEASURE stopped records={last} reason=budget");
            break;
        }
        let began = Instant::now();
        let size = STEP.min(count - start);
        // 每 5 万条报一次：造到一半被时限掐掉也留得下数。
        for half in 0..size.div_ceil(STEP / 2) {
            let chunk = Instant::now();
            let part = (STEP / 2).min(size - half * (STEP / 2));
            last = fill(&stage, &format!("bulk{round}-{half}-"), part);
            println!(
                "MEASURE fill last_offset={last} chunk_ms_avg={:.3} seconds={:.1}",
                chunk.elapsed().as_secs_f64() * 1000.0 / part as f64,
                started.elapsed().as_secs_f64()
            );
        }
        last = measure_round(&stage, last);
        previous = began.elapsed();
    }

    // 一次封存（冻结全部库、导出、封存；夹具的冻结预算和生产一样是 30 秒，不含同步平面）。
    let restarted = stage.restarted();
    restarted.catch_up(&AtomicBool::new(false));
    let started = Instant::now();
    assert_eq!(
        restarted_ask(&restarted, QueueCommand::Schedule),
        QueueAnswer::Started
    );
    restarted.settle();
    let sealed = matches!(
        restarted_ask(&restarted, QueueCommand::NextPending),
        QueueAnswer::Pending(_)
    );
    println!(
        "MEASURE seal records={last} seconds={:.2} sealed={sealed}",
        started.elapsed().as_secs_f64()
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 账本现在有 `last` 条时量一轮，返回量完之后的末条偏移（单次追加那一项自己加了 100 条）。
fn measure_round(stage: &Stage, last: u64) -> u64 {
    // 账本每次读写都要取首尾偏移：同一个库上，旧写法（两个聚合写在一句里）和新写法（各写一句）各跑 20 次取平均。
    let db = rusqlite::Connection::open_with_flags(
        stage.assembled.fixture.root.join("audit/audit.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let timed = |statements: &[&str]| {
        let started = Instant::now();
        for _ in 0..20 {
            for sql in statements {
                db.query_row(sql, [], |row| row.get::<_, Option<i64>>(0))
                    .unwrap();
            }
        }
        started.elapsed().as_secs_f64() * 1000.0 / 20.0
    };
    println!(
        "MEASURE records={last} bounds_ms before={:.3} after={:.3}",
        timed(&["SELECT min(offset), max(offset) FROM audit_events"]),
        timed(&[
            "SELECT min(offset) FROM audit_events",
            "SELECT max(offset) FROM audit_events"
        ])
    );
    drop(db);

    let started = Instant::now();
    let last = fill(stage, &format!("single{last}-"), 100);
    println!(
        "MEASURE records={last} append_ms_avg={:.3}",
        started.elapsed().as_secs_f64() * 1000.0 / 100.0
    );

    // 人侧重启：队列进度从零开始，后台线程把整本账本读到末尾要多久。小页（请求那条路的页大小）和真跑时的大页各量一次。
    let timed = |size: Option<usize>| {
        let restarted = stage.restarted();
        let started = Instant::now();
        match size {
            Some(size) => restarted.catch_up_by(&AtomicBool::new(false), size),
            None => restarted.catch_up(&AtomicBool::new(false)),
        }
        assert_eq!(restarted.scanned_offset(), last);
        started.elapsed().as_secs_f64()
    };
    println!(
        "MEASURE records={last} catch_up_seconds small_page={:.2} real_page={:.2}",
        timed(Some(PAGE)),
        timed(None)
    );
    last
}

fn restarted_ask(
    producer: &std::sync::Arc<crate::backup::producer::BackupProducer>,
    command: QueueCommand,
) -> QueueAnswer {
    let request = studiolo_backup::worker::audit::BackupQueueRequest::new(
        crate::backup::producer::TARGET,
        command,
    );
    request
        .decode_reply(&producer.dispatch(&request.encode().unwrap()).unwrap())
        .unwrap()
}
