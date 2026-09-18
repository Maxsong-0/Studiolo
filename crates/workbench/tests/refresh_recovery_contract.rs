//! 运行者死亡后的恢复契约：等待者被明确告知失败，排队中的派生由等待者接手，
//! 全程不靠超时、不靠睡眠，派生次数来自真实调用计数。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;

use studiolo_workbench::{RefreshGate, RefreshTicket, RefreshWait};

/// 计数用的假派生：只记录被调用次数，不做任何真实工作。
#[derive(Default)]
struct Derivations(AtomicUsize);

impl Derivations {
    fn run(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// 等待到给定请求数为止，用条件变量而不是睡眠或超时。
#[derive(Default)]
struct Arrivals {
    count: Mutex<usize>,
    changed: Condvar,
}

impl Arrivals {
    fn arrive(&self) {
        let mut count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        *count += 1;
        drop(count);
        self.changed.notify_all();
    }

    fn wait_for(&self, target: usize) {
        let mut count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        while *count < target {
            count = self
                .changed
                .wait(count)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

/// 等待者的标准循环：读不到结论就先试着接手，接不到才睡。
fn wait_or_take_over(
    gate: &Mutex<RefreshGate>,
    settled: &Condvar,
    derivations: &Derivations,
    ticket: RefreshTicket,
) -> RefreshWait {
    let mut state = gate.lock().unwrap_or_else(|error| error.into_inner());
    loop {
        let observed = state.observe(ticket);
        if observed != RefreshWait::Running {
            return observed;
        }
        if let Some(run) = state.claim() {
            drop(state);
            derivations.run();
            state = gate.lock().unwrap_or_else(|error| error.into_inner());
            state.settle(run, Ok(()));
            settled.notify_all();
            continue;
        }
        state = settled
            .wait(state)
            .unwrap_or_else(|error| error.into_inner());
    }
}

#[test]
fn a_ticket_covered_by_an_abandoned_run_is_told_it_failed_instead_of_waiting_forever() {
    let gate = Mutex::new(RefreshGate::new());
    let settled = Condvar::new();
    let derivations = Derivations::default();
    let arrivals = Arrivals::default();

    let admission = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .request("http")
        .expect("空闲闸门受理请求");
    let doomed = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .claim()
        .expect("空闲闸门上的认领必定成功");

    let observed = thread::scope(|scope| {
        let waiter = scope.spawn(|| {
            arrivals.arrive();
            wait_or_take_over(&gate, &settled, &derivations, admission.ticket())
        });
        arrivals.wait_for(1);
        // 运行者死亡：派生句柄随线程一起消失，永远不会被结算。
        let _never_settled = doomed;
        let declared = gate
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .fail_abandoned_run("刷新线程已死亡");
        assert!(declared, "在途派生被判定为遗弃");
        settled.notify_all();
        waiter.join().expect("等待者线程正常结束")
    });

    assert_eq!(observed, RefreshWait::Failed("刷新线程已死亡"));
    assert_eq!(derivations.calls(), 0, "遗弃不产生新的派生");
    let stats = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stats();
    assert_eq!(stats.runs_abandoned, 1);
    assert_eq!(stats.runs_failed, 1);
}

#[test]
fn eight_waiters_of_an_abandoned_run_take_the_queued_derivation_over_exactly_once() {
    let gate = Mutex::new(RefreshGate::new());
    let settled = Condvar::new();
    let derivations = Derivations::default();
    let arrivals = Arrivals::default();

    gate.lock()
        .unwrap_or_else(|error| error.into_inner())
        .request("http")
        .expect("空闲闸门受理请求");
    let doomed = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .claim()
        .expect("空闲闸门上的认领必定成功");

    let results = thread::scope(|scope| {
        let waiters: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let admission = gate
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .request("watcher")
                        .expect("在途期间仍然受理");
                    assert!(!admission.starts_run(), "在途期间不得出现第二个运行者");
                    arrivals.arrive();
                    let ticket = admission.ticket();
                    (
                        ticket,
                        wait_or_take_over(&gate, &settled, &derivations, ticket),
                    )
                })
            })
            .collect();

        arrivals.wait_for(8);
        // 运行者死亡：派生句柄随线程一起消失，永远不会被结算。
        let _never_settled = doomed;
        assert!(
            gate.lock()
                .unwrap_or_else(|error| error.into_inner())
                .fail_abandoned_run("刷新线程已死亡"),
            "在途派生被判定为遗弃"
        );
        settled.notify_all();

        waiters
            .into_iter()
            .map(|waiter| waiter.join().expect("等待者线程正常结束"))
            .collect::<Vec<_>>()
    });

    assert!(
        results
            .iter()
            .all(|(_, observed)| *observed == RefreshWait::Completed),
        "接手之后每个等待者都读到成功，没有人被永久挂起"
    );
    let distinct: std::collections::BTreeSet<_> =
        results.iter().map(|(ticket, _)| *ticket).collect();
    assert_eq!(distinct.len(), 1, "八个等待者共享同一张排队票");
    assert_eq!(derivations.calls(), 1, "排队中的派生只被跑了一遍");

    let gate = gate.into_inner().unwrap_or_else(|error| error.into_inner());
    let stats = gate.stats();
    assert_eq!(stats.admitted, 9);
    assert_eq!(stats.runs_started, 2, "首次认领加一次接手");
    assert_eq!(stats.runs_abandoned, 1);
    assert_eq!(stats.settles_ignored, 0);
    assert!(!gate.is_busy());
}
