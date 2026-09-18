//! 刷新闸门的失败、遗弃与关闭契约；全部为单线程状态转换，结论来自真实调用计数。

use studiolo_workbench::{RefreshGate, RefreshTicket, RefreshWait};

/// 跑一次「空闲 → 认领 → 结算」，返回本次派生覆盖的票。
fn run_once(
    gate: &mut RefreshGate,
    reason: &'static str,
    outcome: Result<(), &'static str>,
) -> RefreshTicket {
    let admission = gate.request(reason).expect("闸门未关闭时受理请求");
    assert!(admission.starts_run(), "空闲闸门要求请求方去认领派生");
    let run = gate.claim().expect("空闲闸门上的认领必定成功");
    let covers = run.covers();
    assert_eq!(covers, admission.ticket());
    gate.settle(run, outcome);
    covers
}

#[test]
fn a_failed_derivation_reports_its_own_reason_to_every_ticket_it_covered() {
    let mut gate = RefreshGate::new();
    let first = gate.request("http").expect("空闲闸门受理请求");
    let run = gate.claim().expect("空闲闸门上的认领必定成功");
    let joined = gate.request("watcher").expect("在途期间仍然受理");
    gate.settle(run, Err("读取本代文档失败"));

    assert_eq!(
        gate.observe(first.ticket()),
        RefreshWait::Failed("读取本代文档失败")
    );
    // 排队的那张票不由这次派生覆盖，失败不会被提前当成它的结论。
    assert_eq!(gate.observe(joined.ticket()), RefreshWait::Running);
    assert_eq!(gate.stats().runs_failed, 1);
    assert_eq!(gate.stats().runs_abandoned, 0);
}

#[test]
fn a_failure_leaves_no_intermediate_state_and_the_next_derivation_clears_it() {
    let mut gate = RefreshGate::new();
    let failed = run_once(&mut gate, "http", Err("读取本代文档失败"));
    assert!(!gate.is_busy(), "失败结算之后闸门既不在途也不排队");
    assert!(!gate.is_closed(), "失败不锁死闸门");

    let recovered = run_once(&mut gate, "http", Ok(()));
    assert!(recovered > failed, "恢复靠重新跑一遍完整派生，不复用旧票号");
    assert_eq!(gate.observe(recovered), RefreshWait::Completed);
    let stats = gate.stats();
    assert_eq!(stats.runs_started, 2);
    assert_eq!(stats.runs_failed, 1);
    assert_eq!(stats.settles_ignored, 0);
}

#[test]
fn an_abandoned_run_fails_its_tickets_and_frees_the_slot_for_the_queued_derivation() {
    let mut gate = RefreshGate::new();
    let first = gate.request("http").expect("空闲闸门受理请求");
    let abandoned = gate.claim().expect("空闲闸门上的认领必定成功");
    let queued = gate.request("watcher").expect("在途期间仍然受理");

    assert!(gate.fail_abandoned_run("刷新线程已死亡"));
    assert_eq!(
        gate.observe(first.ticket()),
        RefreshWait::Failed("刷新线程已死亡")
    );
    assert!(gate.is_busy(), "排队位不因遗弃而丢失");

    let takeover = gate.claim().expect("在途位已释放，排队中的派生可以被接手");
    assert_eq!(takeover.covers(), queued.ticket());
    assert_eq!(takeover.reason(), "watcher");

    // 被判定为遗弃的运行者若「死而复生」，它的迟到结算不得抹掉已交付的结论。
    gate.settle(abandoned, Ok(()));
    assert_eq!(
        gate.observe(first.ticket()),
        RefreshWait::Failed("刷新线程已死亡")
    );
    assert_eq!(gate.observe(queued.ticket()), RefreshWait::Running);

    gate.settle(takeover, Ok(()));
    assert_eq!(gate.observe(queued.ticket()), RefreshWait::Completed);
    let stats = gate.stats();
    assert_eq!(stats.runs_started, 2);
    assert_eq!(stats.runs_failed, 1);
    assert_eq!(stats.runs_abandoned, 1);
    assert_eq!(stats.settles_ignored, 1);
}

#[test]
fn declaring_an_abandonment_without_a_run_in_flight_changes_nothing() {
    let mut gate = RefreshGate::new();
    assert!(!gate.fail_abandoned_run("刷新线程已死亡"));
    let covered = run_once(&mut gate, "http", Ok(()));
    assert_eq!(gate.observe(covered), RefreshWait::Completed);
    assert_eq!(gate.stats().runs_abandoned, 0);
    assert_eq!(gate.stats().runs_failed, 0);
}

#[test]
fn a_runner_that_stops_claiming_only_delays_the_queued_derivation_for_the_next_request() {
    let mut gate = RefreshGate::new();
    let first = gate.request("http").expect("空闲闸门受理请求");
    let run = gate.claim().expect("空闲闸门上的认领必定成功");
    let queued = gate.request("watcher").expect("在途期间仍然受理");
    // 运行者结算后就走人，没有按协议继续认领排队中的那次派生。
    gate.settle(run, Ok(()));
    assert_eq!(gate.observe(first.ticket()), RefreshWait::Completed);
    assert_eq!(gate.observe(queued.ticket()), RefreshWait::Running);

    let next = gate.request("http").expect("闸门没有被卡死");
    assert!(next.starts_run(), "下一个请求被要求接手，而不是继续空等");
    assert_eq!(next.ticket(), queued.ticket(), "接手的是同一张排队票");
    let taken = gate.claim().expect("排队中的派生仍然可以被认领");
    gate.settle(taken, Ok(()));
    assert_eq!(gate.observe(queued.ticket()), RefreshWait::Completed);
    assert_eq!(gate.stats().runs_started, 2);
}

#[test]
fn closing_the_gate_refuses_new_requests_and_tells_unfinished_tickets_it_will_not_answer() {
    let mut gate = RefreshGate::new();
    let first = gate.request("http").expect("空闲闸门受理请求");
    let run = gate.claim().expect("空闲闸门上的认领必定成功");
    let queued = gate.request("watcher").expect("在途期间仍然受理");

    gate.close();
    assert!(gate.is_closed());
    assert_eq!(gate.observe(first.ticket()), RefreshWait::Closed);
    assert_eq!(gate.observe(queued.ticket()), RefreshWait::Closed);
    assert!(gate.request("http").is_err(), "关闭后一律拒绝受理");
    assert!(gate.claim().is_none(), "关闭后不再认领排队工作");

    // 关闭前已认领的派生仍须结算；结算不会让关闭后的票重新变成完成。
    gate.settle(run, Ok(()));
    assert_eq!(gate.observe(first.ticket()), RefreshWait::Closed);
    assert!(!gate.is_busy());
}

#[test]
fn exhausting_the_ticket_space_closes_the_gate_instead_of_reusing_a_ticket() {
    let mut gate = RefreshGate::resuming_from(u64::MAX);
    assert!(!gate.is_closed());
    assert!(gate.request("http").is_err(), "票号触顶时停止受理");
    assert!(gate.is_closed(), "触顶后主动关闭，不回绕重用票号");
    assert_eq!(gate.stats().admitted, 0, "被拒的请求不计入受理数");
}

#[test]
fn resuming_from_a_ticket_issues_strictly_later_tickets() {
    let mut gate = RefreshGate::resuming_from(41);
    let admission = gate.request("http").expect("重建后的闸门受理请求");
    assert_eq!(admission.ticket().value(), 42);
    assert!(admission.starts_run());
}
