//! 封存的冻结窗口只定格：拷贝、整本验链、过滤、发布都在放开屏障之后做（BUG-045）。
//!
//! 以前整次封存都在窗口里：人侧写请求要取完整操作许可，得等整次封存做完，最多等 5 秒就报错；
//! 169 的账本 70 万条时一次封存要一分多钟，备份永远封不成，每次还挡写一分钟。
use super::{producer::Stage, producer_catch_up::fill, *};
use std::sync::atomic::{AtomicBool, Ordering};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_backup::worker::audit::{QueueAnswer, QueueCommand};

/// 账本放这么多条：放开之后的拷贝、摘要和整本验链明显比窗口里那几步长，两段的用时分得开。
const RECORDS: usize = 3000;

fn event(label: &str) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("board:task.reconcile:{label}")).unwrap(),
        AuditPhase::Intent,
        "task.reconcile",
        SanitizedPayload::try_from(serde_json::json!({"request_hash": "0".repeat(64)})).unwrap(),
    )
}

/// 封存进行中一直试着做人侧写操作（取完整操作许可、追加一条账本）：只在冻结窗口里拿不到，
/// 窗口之外随时拿得到。最长的一段拿不到，要比整次封存短得多。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writes_wait_only_for_the_freeze_window_not_for_the_whole_seal() {
    let stage = Stage::new(true).await;
    fill(&stage, "window-", RECORDS);
    let human = stage.assembled.human.clone();
    let done = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    let producer = stage.producer.clone();
    let finished = done.clone();
    let waiter = std::thread::spawn(move || {
        producer.settle();
        finished.store(true, Ordering::Release);
    });
    let (mut last_write, mut longest, mut writes) = (Instant::now(), Duration::ZERO, 0_u32);
    while !done.load(Ordering::Acquire) {
        if let Some(permit) = human.try_enter_operation().unwrap() {
            human.append(event(&format!("during-{writes}"))).unwrap();
            drop(permit);
            writes += 1;
            longest = longest.max(last_write.elapsed());
            last_write = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    longest = longest.max(last_write.elapsed());
    waiter.join().unwrap();
    let total = started.elapsed();
    println!(
        "封存共 {} 毫秒，写操作最长拿不到许可 {} 毫秒，封存期间写成 {writes} 次",
        total.as_millis(),
        longest.as_millis()
    );
    assert!(
        matches!(
            stage.ask(QueueCommand::NextPending),
            QueueAnswer::Pending(_)
        ),
        "封存要成功"
    );
    assert!(
        longest * 3 < total,
        "写操作只该等冻结窗口：最长等了 {longest:?}，整次封存 {total:?}"
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 关停只等冻结窗口放开（`settle_window`），不等放开之后的拷贝和发布；那一段进程退出时丢下，
/// 留下的暂存由下次启动回收（和「封存做到一半被杀」同一条路）。用例里让封存停在放开之后，
/// 关停这时要能返回、写操作拿得到许可、准入还没写；放行之后封存照样做完。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_the_freeze_window_but_not_for_the_copy_after_it() {
    let stage = Stage::new(true).await;
    fill(&stage, "shutdown-", 100);
    let (open, gate) = std::sync::mpsc::channel();
    *stage.producer.hold_after_release.lock().unwrap() = Some(gate);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    let producer = stage.producer.clone();
    let (settled, window) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        producer.settle_window();
        let _ = settled.send(());
    });
    assert!(
        window.recv_timeout(Duration::from_secs(60)).is_ok(),
        "封存停在放开之后时，关停要能返回"
    );
    assert!(
        stage
            .assembled
            .human
            .try_enter_operation()
            .unwrap()
            .is_some(),
        "窗口放开之后马上拿得到操作许可"
    );
    stage.assembled.writable(true);
    assert_eq!(
        stage.ask(QueueCommand::NextPending),
        QueueAnswer::Idle,
        "封存还停在放开之后，准入还没写"
    );
    open.send(()).unwrap();
    stage.producer.settle();
    assert!(
        matches!(
            stage.ask(QueueCommand::NextPending),
            QueueAnswer::Pending(_)
        ),
        "放行之后封存照样做完"
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}
