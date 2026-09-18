//! 刷新闸门的行为、失败与恢复契约；并发证据来自真实线程与真实调用计数。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;

use studiolo_workbench::{RefreshGate, RefreshWait};

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

#[test]
fn an_idle_gate_admits_the_first_request_as_the_runner_and_coalesces_the_rest() {
    let mut gate = RefreshGate::new();
    let first = gate.request("http").expect("空闲闸门受理请求");
    assert!(first.starts_run());
    let run = gate.claim().expect("运行者认领排队中的派生");
    assert_eq!(run.covers(), first.ticket());

    let second = gate.request("watcher").expect("在途期间仍然受理");
    let third = gate.request("periodic").expect("在途期间仍然受理");
    assert!(!second.starts_run());
    assert!(!third.starts_run());
    assert_eq!(second.ticket(), third.ticket());
    assert!(second.ticket() > first.ticket());

    gate.settle(run, Ok(()));
    assert_eq!(gate.observe(first.ticket()), RefreshWait::Completed);
    assert_eq!(gate.observe(second.ticket()), RefreshWait::Running);

    let queued = gate.claim().expect("排队位换来第二次派生");
    assert_eq!(queued.covers(), second.ticket());
    assert_eq!(queued.reason(), "watcher");
    gate.settle(queued, Ok(()));
    assert_eq!(gate.observe(third.ticket()), RefreshWait::Completed);
    assert!(gate.claim().is_none());
    assert!(!gate.is_busy());

    let stats = gate.stats();
    assert_eq!(stats.admitted, 3);
    assert_eq!(stats.runs_started, 2);
    assert_eq!(stats.coalesced(), 1);
}

#[test]
fn thirty_two_concurrent_requests_cost_exactly_one_extra_derivation() {
    let gate = Mutex::new(RefreshGate::new());
    let settled = Condvar::new();
    let derivations = Derivations::default();
    let arrivals = Arrivals::default();

    // 先让一次派生进入在途，后到的并发请求才有「合流」可言。
    let opening = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .request("http")
        .expect("空闲闸门受理请求");
    assert!(opening.starts_run());
    let first = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .claim()
        .expect("运行者认领第一次派生");

    let tickets = thread::scope(|scope| {
        let waiters: Vec<_> = (0..32)
            .map(|_| {
                scope.spawn(|| {
                    let admission = gate
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .request("http")
                        .expect("在途期间仍然受理");
                    arrivals.arrive();
                    assert!(!admission.starts_run(), "在途期间不得出现第二个运行者");
                    let ticket = admission.ticket();
                    let mut state = gate.lock().unwrap_or_else(|error| error.into_inner());
                    while state.observe(ticket) == RefreshWait::Running {
                        state = settled
                            .wait(state)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                    (ticket, state.observe(ticket))
                })
            })
            .collect();

        // 32 个请求全部到达之后，第一次派生才结束；合流窗口因此是确定的。
        arrivals.wait_for(32);
        derivations.run();
        let mut run = Some(first);
        while let Some(current) = run.take() {
            gate.lock()
                .unwrap_or_else(|error| error.into_inner())
                .settle(current, Ok(()));
            settled.notify_all();
            run = gate
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .claim();
            if run.is_some() {
                derivations.run();
            }
        }
        settled.notify_all();

        waiters
            .into_iter()
            .map(|waiter| waiter.join().expect("等待者线程正常结束"))
            .collect::<Vec<_>>()
    });

    assert_eq!(derivations.calls(), 2, "33 次请求只换来两次真实派生");
    assert!(
        tickets
            .iter()
            .all(|(_, state)| *state == RefreshWait::Completed)
    );
    let distinct: std::collections::BTreeSet<_> =
        tickets.iter().map(|(ticket, _)| *ticket).collect();
    assert_eq!(distinct.len(), 1, "并发请求共享同一张票");

    let stats = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stats();
    assert_eq!(stats.admitted, 33);
    assert_eq!(stats.runs_started, 2);
    assert_eq!(stats.coalesced(), 31);
    assert_eq!(u64::try_from(derivations.calls()), Ok(stats.runs_started));
}

#[test]
fn a_free_race_of_requests_never_starts_more_than_two_derivations_at_a_time() {
    let gate = Mutex::new(RefreshGate::new());
    let settled = Condvar::new();
    let derivations = Derivations::default();

    thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                let admission = gate
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .request("http")
                    .expect("闸门未关闭时受理请求");
                let ticket = admission.ticket();
                if admission.starts_run() {
                    loop {
                        let run = gate
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .claim();
                        let Some(run) = run else { break };
                        derivations.run();
                        gate.lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .settle(run, Ok(()));
                        settled.notify_all();
                    }
                    settled.notify_all();
                }
                let mut state = gate.lock().unwrap_or_else(|error| error.into_inner());
                while state.observe(ticket) == RefreshWait::Running {
                    state = settled
                        .wait(state)
                        .unwrap_or_else(|error| error.into_inner());
                }
                assert_eq!(state.observe(ticket), RefreshWait::Completed);
            });
        }
    });

    let stats = gate
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .stats();
    assert_eq!(stats.admitted, 16);
    assert!(stats.runs_started <= 16);
    assert_eq!(u64::try_from(derivations.calls()), Ok(stats.runs_started));
    assert!(
        !gate
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_busy()
    );
}
