//! 封存冻结期间，看板租约巡检这一轮跳过：不等 5 秒、不当故障，巡检循环照常往下走（BUG-050）。
//!
//! 169 上冻结窗口 26 秒：巡检取完整操作许可等满 5 秒失败，整个巡检循环结束，人侧照「board 一侧先结束」
//! 走完整关停、以退出码 1 退出，封存后半段随进程丢掉。冻结以外的错误照旧失败关闭。
use super::{fixture::Assembled, *};
use crate::board::{MAX_FROZEN, MaintenanceFault, MaintenanceTick};
use studiolo_audit_ledger::AuditQuiescence;

/// 冻结中的一轮巡检要当场返回；改前要等满 5 秒。
const PROMPT: Duration = Duration::from_secs(1);
/// 用例里的巡检间隔：几十毫秒一轮，冻结期间跳过很多轮。
const INTERVAL: Duration = Duration::from_millis(20);
/// 冻结占着的时长：比取许可最多等的 5 秒长，改前巡检在这期间就报错结束了。
const LONG_FREEZE: Duration = Duration::from_millis(5_500);
/// 用例里的连续冻结封顶时长：比冻结短得多，巡检循环该在冻结放开之前结束。
const SHORT_CAP: Duration = Duration::from_millis(300);
/// 循环该结束的情形下，最多等它这么久；远比间隔和封顶长，不会因为机器慢误报。
const ENDS_WITHIN: Duration = Duration::from_secs(5);

fn plane(assembled: &Assembled) -> Arc<BoardPlane> {
    assembled.inputs.board.clone().expect("装配里有黑板")
}

/// 像封存那样占住人侧账本的排空许可。
fn freeze(human: &SqliteAuditLedger) -> AuditQuiescence {
    human
        .quiesce_operations(Instant::now() + Duration::from_secs(2))
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tick_during_the_freeze_is_skipped_at_once() {
    let assembled = Assembled::new().await;
    let plane = plane(&assembled);
    assert_eq!(plane.maintain_once(None), Ok(MaintenanceTick::Done));
    let frozen = freeze(&assembled.human);
    let started = Instant::now();
    let tick = plane.maintain_once(None);
    let waited = started.elapsed();
    drop(frozen);
    assert_eq!(
        tick,
        Ok(MaintenanceTick::SkippedFrozen),
        "冻结中的一轮巡检要跳过，不当故障"
    );
    assert!(
        waited < PROMPT,
        "冻结中的一轮巡检等了 {} 毫秒",
        waited.as_millis()
    );
    assert_eq!(plane.maintain_once(None), Ok(MaintenanceTick::Done));
    assembled.workbench.shutdown().await.unwrap();
}

/// 冻结比 5 秒长：期间跳过很多轮，巡检循环一直在；放开之后照常，收到停止就正常结束。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_maintenance_loop_keeps_going_through_a_freeze() {
    let assembled = Assembled::new().await;
    let plane = plane(&assembled);
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let frozen = freeze(&assembled.human);
    let maintenance = tokio::spawn(plane.maintain_every(INTERVAL, MAX_FROZEN, stopped, None));
    tokio::time::sleep(LONG_FREEZE).await;
    assert!(!maintenance.is_finished(), "冻结期间巡检循环结束了");
    drop(frozen);
    tokio::time::sleep(INTERVAL * 5).await;
    assert!(!maintenance.is_finished(), "放开之后巡检循环结束了");
    stop.send(true).unwrap();
    let ended = tokio::time::timeout(PROMPT, maintenance)
        .await
        .expect("收到停止之后巡检没及时结束")
        .unwrap();
    assert_eq!(ended, Ok(()));
    assembled.workbench.shutdown().await.unwrap();
}

/// 冻结一直不放（封存卡死）：连续跳过超过封顶时长，巡检循环按「备份冻结超过上限」结束、失败关闭，
/// 不让人侧活着却一直写不进去。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_freeze_that_never_lets_go_ends_the_loop() {
    let assembled = Assembled::new().await;
    let plane = plane(&assembled);
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let frozen = freeze(&assembled.human);
    let maintenance = tokio::spawn(plane.maintain_every(INTERVAL, SHORT_CAP, stopped, None));
    let ended = tokio::time::timeout(ENDS_WITHIN, maintenance).await;
    drop(frozen);
    let ended = ended.expect("冻结超过封顶时长，巡检循环没结束").unwrap();
    assert_eq!(ended, Err(MaintenanceFault::FrozenTooLong));
    assembled.workbench.shutdown().await.unwrap();
}

/// 冻结以外的故障（控制目录权限被改）结束的是整个巡检循环，不只是这一轮：人侧照旧走完整关停。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fault_ends_the_maintenance_loop() {
    let assembled = Assembled::new().await;
    let plane = plane(&assembled);
    let (_stop, stopped) = tokio::sync::watch::channel(false);
    let control = assembled.fixture.root.join("control");
    fs::set_permissions(&control, fs::Permissions::from_mode(0o750)).unwrap();
    let maintenance = tokio::spawn(plane.maintain_every(INTERVAL, MAX_FROZEN, stopped, None));
    let ended = tokio::time::timeout(ENDS_WITHIN, maintenance).await;
    fs::set_permissions(&control, fs::Permissions::from_mode(0o700)).unwrap();
    let ended = ended.expect("故障之后巡检循环没结束").unwrap();
    assert_eq!(ended, Err(MaintenanceFault::Storage));
    assembled.workbench.shutdown().await.unwrap();
}

/// 准入文件权限被改（不是冻结）：照旧算审计不可用、失败关闭，不当成「冻结中」跳过。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unsafe_audit_gate_is_a_fault_not_a_skip() {
    let assembled = Assembled::new().await;
    let plane = plane(&assembled);
    assert_eq!(plane.maintain_once(None), Ok(MaintenanceTick::Done));
    let gate = assembled
        .fixture
        .root
        .join("audit")
        .join("checkpoint-operations.lock");
    fs::set_permissions(&gate, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(plane.maintain_once(None), Err(MaintenanceFault::Audit));
    fs::set_permissions(&gate, fs::Permissions::from_mode(0o600)).unwrap();
    assembled.workbench.shutdown().await.unwrap();
}

/// 控制目录权限被改：照旧失败关闭，原因是固定类别。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_control_directory_is_still_a_fault() {
    let assembled = Assembled::new().await;
    let plane = plane(&assembled);
    let control = assembled.fixture.root.join("control");
    fs::set_permissions(&control, fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(plane.maintain_once(None), Err(MaintenanceFault::Storage));
    fs::set_permissions(&control, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(plane.maintain_once(None), Ok(MaintenanceTick::Done));
    assembled.workbench.shutdown().await.unwrap();
}
