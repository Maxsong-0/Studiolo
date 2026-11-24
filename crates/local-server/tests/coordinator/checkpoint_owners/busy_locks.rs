//! 封存开头、冻结时要的几把进程内锁被人拿着：短暂拿着的等一小会儿照样做成，一直不放的照旧失败（BUG-055）。
//!
//! 169 上 2026-11-24 06:40Z 一次定时封存一开始就失败：「备份封存卡在：owner Control 不可用（已走：无）」——
//! 取控制库固定源时对黑板引擎锁「占着就放弃」，撞上就白等 10 分钟。
use super::{fixture::Assembled, *};
use crate::bounded_lock::BRIEF;
use std::{sync::mpsc, thread};

/// 短暂持有者占着的时长：比以前「当场放弃」长得多，比等锁上限短得多。
const SHORT: Duration = Duration::from_millis(200);
/// 卡死的持有者占着的时长：比等锁上限长，测试机慢也等得到上限先到。
const STUCK: Duration = Duration::from_secs(3);

/// 黑板引擎锁被巡检那样拿着：封存照样能固定源（改前就在这一步失败）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_board_engine_does_not_stop_the_seal() {
    let assembled = Assembled::new().await;
    let plane = assembled.inputs.board.clone().expect("装配里有黑板");
    let engine = plane.hold_engine();
    let owners = assembled.inputs.bind();
    assert!(
        owners.is_ok(),
        "引擎锁被占着，封存没能固定源：{:?}",
        owners.err()
    );
    drop(engine);
    assembled.workbench.shutdown().await.unwrap();
}

/// 写者的两把锁被短暂拿着（监视回调登记失效、网页读一篇笔记）：冻结等得到，照样做成。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_writer_lock_held_briefly_is_waited_for() {
    for lock in ["control", "owner"] {
        let f = Setup::new(true).await;
        let holder = hold_writer(&f, lock, SHORT);
        let frozen = f
            .workbench
            .freeze_checkpoint(Instant::now() + Duration::from_secs(20));
        assert!(
            frozen.is_ok(),
            "{lock} 短暂被占，冻结没做成：{:?}",
            frozen.err()
        );
        drop(frozen);
        holder.join().unwrap();
        f.workbench.shutdown().await.unwrap();
    }
}

/// 写者的锁一直不放：等满上限照旧按「写者不可用」失败，不会一直等到冻结预算用完。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_writer_lock_that_is_never_let_go_still_fails_in_time() {
    for lock in ["control", "owner"] {
        let f = Setup::new(true).await;
        let holder = hold_writer(&f, lock, STUCK);
        let started = Instant::now();
        let mut steps = crate::checkpoint::StepTimes::start();
        let result = f.owners.freeze_timed(
            f.request(30_000),
            &f.key,
            BackupTargetKind::LocalTemporaryRepository,
            f.targets(),
            &mut steps,
        );
        let waited = started.elapsed();
        assert_eq!(
            result.err(),
            Some(CheckpointError::OwnerUnavailable {
                owner: Owner::Broker
            }),
            "{lock} 一直被占，冻结不该成"
        );
        assert!(
            waited >= BRIEF,
            "{lock} 没等满上限：{} 毫秒",
            waited.as_millis()
        );
        assert!(
            waited < STUCK,
            "{lock} 等过了头：{} 毫秒",
            waited.as_millis()
        );
        holder.join().unwrap();
        // 失败之后写者还在原处，下一次照样冻得住。
        assert!(
            f.workbench
                .freeze_checkpoint(Instant::now() + Duration::from_secs(20))
                .is_ok()
        );
        f.workbench.shutdown().await.unwrap();
    }
}

fn hold_writer(f: &Setup, lock: &'static str, hold: Duration) -> thread::JoinHandle<()> {
    let core = f.workbench.core.clone();
    let (taken, took) = mpsc::channel();
    let holder = thread::spawn(move || match lock {
        "control" => {
            let guard = core.control.lock().unwrap();
            taken.send(()).unwrap();
            thread::sleep(hold);
            drop(guard);
        }
        _ => {
            let guard = core.owner.lock().unwrap();
            taken.send(()).unwrap();
            thread::sleep(hold);
            drop(guard);
        }
    });
    took.recv().unwrap();
    holder
}
