//! 第 10 步原键核对（评审 P1-1）：真实临时 `BridgeState` ＋ 假核对端口。证明 ① canonical 未知经原键
//! 重放核成提交、下一轮闭合；② 未碰后端即明确无效果、计划终结、基线不前进、下一轮重新提出同一动作；
//! ③ 镜像目标三态判定，核不出仍未知且未闭合序号仍在；④ 只按原键问未知项一次、已知原键绝不再碰。

use super::reconcile_support::{
    FakePort, commit, committed, common, counts, input, issue_all, present, record, run, settle,
    status, store, version, view,
};
use studiolo_local_server::sync_plane::driver::reconcile::{
    MirrorTarget, mirror_verdict, replay_outcome,
};
use studiolo_local_server::sync_plane::job_backend::{
    SeamVerdict, first_attempt_outcome, not_attempted,
};
use studiolo_sync_bridge::executor::{JobOutcome, NoEffectCause};
use studiolo_sync_bridge::sqlite::ActionStatus;
use studiolo_sync_bridge::{Change, Decision, Presence, Side, plan};

const KEY: &str = "bridge-v1-fixed-key";

#[test]
fn canonical_unknown_replays_to_the_original_receipt_and_closes_next_round() {
    let mut store = store();
    let original = input(&store, &[("notes/a.md", Presence::Missing, present(9))]);
    let issued = issue_all(&mut store, &original);
    let (action, job) = &issued[0];
    assert_eq!(action.key.target, Side::Canonical);
    let mut port = FakePort::new();
    port.recorded = Some(commit(action));
    assert_eq!(run(&mut port, &mut store), counts(1, 0));
    // 按原键、原请求摘要问一次；有原回执，接缝不会第二次安装。
    assert_eq!(port.replayed, vec![job.clone()]);
    assert_eq!(port.installs, 0);
    assert!(committed(&status(&store, action)));
    // 下一轮：已无未知，执行器闭合并推进共同基线。
    let closing = common(&store, &original, &present(9));
    settle(&mut store, original.plan_sequence, Some(closing));
    assert_eq!(view(&store).pending_sequence, None);
    assert_eq!(view(&store).baseline.version, 1);
}

#[test]
fn not_attempted_is_no_effect_settles_without_baseline_and_is_proposed_again() {
    // 首次执行：未碰 Broker 就失败，明确无效果而不是未知。
    let no_effect = not_attempted(KEY);
    let cause = NoEffectCause::CancelledBeforeInstall;
    assert!(matches!(&no_effect, JobOutcome::NoEffect { reason, .. } if *reason == cause));
    assert_eq!(
        first_attempt_outcome(SeamVerdict::NotAttempted, KEY),
        no_effect
    );
    assert_eq!(
        first_attempt_outcome(SeamVerdict::Unknown, KEY),
        JobOutcome::Unknown
    );
    // 重开核对：写者缺席时同一分类什么也没证明；接得上才是 Broker 对同一请求确定拒收。
    assert_eq!(
        replay_outcome(SeamVerdict::NotAttempted, false, KEY),
        JobOutcome::Unknown
    );
    assert_eq!(
        replay_outcome(SeamVerdict::NotAttempted, true, KEY),
        no_effect
    );

    let mut store = store();
    let observed = [("notes/a.md", Presence::Missing, present(9))];
    let original = input(&store, &observed);
    let issued = issue_all(&mut store, &original);
    let (action, job) = &issued[0];
    let outcome = first_attempt_outcome(SeamVerdict::NotAttempted, &job.external_key);
    record(&mut store, &original, job, &outcome);
    settle(&mut store, original.plan_sequence, None);
    assert_eq!(view(&store).pending_sequence, None);
    assert_eq!(view(&store).baseline.version, 0, "无效果不推进共同基线");
    // 下一轮同样的两侧观察：新序号重新提出同一动作。
    let next = input(&store, &observed);
    assert_eq!(next.plan_sequence, original.plan_sequence + 1);
    let Decision::Ready(planned) = plan(next, None).expect("可规划") else {
        panic!("不应熔断");
    };
    let again = &planned.actions()[0];
    assert_eq!(again.key.path, action.key.path);
    assert_eq!(again.key.target, action.key.target);
    assert_eq!(again.key.operation, action.key.operation);
    assert_eq!(again.change, action.change);
}

#[test]
fn mirror_target_source_commits_expected_is_not_installed_and_other_stays_unknown() {
    let mut store = store();
    let original = input(&store, &[("notes/a.md", present(9), Presence::Missing)]);
    let issued = issue_all(&mut store, &original);
    let (create, _) = &issued[0];
    assert_eq!(create.key.target, Side::Mirror);
    let mut replace = create.clone();
    replace.change = Change::Replace {
        source: version(9),
        target_expected: version(1),
    };
    let mut delete = create.clone();
    delete.change = Change::Delete {
        target_expected: version(1),
    };
    let source = JobOutcome::Committed {
        new_revision: Some([9; 32]),
        content_hash: Some([9; 32]),
        bytes: Some(10),
        receipt_id: format!("mirror:{}", "09".repeat(32)),
    };
    let deleted = JobOutcome::Committed {
        new_revision: None,
        content_hash: None,
        bytes: None,
        receipt_id: format!("mirror-deleted:{KEY}"),
    };
    let untouched = JobOutcome::NoEffect {
        reason: NoEffectCause::CancelledBeforeInstall,
        terminal_id: format!("not-installed:{KEY}"),
    };
    let unknown = JobOutcome::Unknown;
    use MirrorTarget::{Expected, Missing, Other, Source};
    for (action, target, want) in [
        (create, Source, &source),
        (create, Missing, &untouched),
        (create, Expected, &unknown),
        (create, Other, &unknown),
        (&replace, Source, &source),
        (&replace, Expected, &untouched),
        (&replace, Missing, &unknown),
        (&replace, Other, &unknown),
        (&delete, Missing, &deleted),
        (&delete, Expected, &untouched),
        (&delete, Source, &unknown),
        (&delete, Other, &unknown),
    ] {
        let got = mirror_verdict(action, target, KEY);
        assert_eq!(&got, want, "{:?} × {target:?}", action.change);
    }
    // 既非源也非前置：仍未知，原键仍 Issued，未闭合序号仍在（`unresolved` 据此返回它）。
    let mut port = FakePort::new();
    assert_eq!(run(&mut port, &mut store), counts(0, 1));
    assert_eq!(status(&store, create), ActionStatus::OutcomeUnknown);
    assert_eq!(view(&store).pending_sequence, Some(original.plan_sequence));
    // 已是源版本但读完之后静止失效：本轮镜像结论作废。
    port.mirror = MirrorTarget::Source;
    port.quiescent = false;
    assert_eq!(run(&mut port, &mut store), counts(0, 1));
    assert_eq!(status(&store, create), ActionStatus::OutcomeUnknown);
    // 静止成立：已是源版本即已提交。
    port.quiescent = true;
    assert_eq!(run(&mut port, &mut store), counts(1, 0));
    assert!(committed(&status(&store, create)));
    assert_eq!(port.probed, 3);
}

#[test]
fn reconcile_asks_each_unknown_key_once_never_touches_known_keys_and_never_reinstalls() {
    let mut store = store();
    let items = [
        ("notes/a.md", Presence::Missing, present(9)),
        ("notes/b.md", Presence::Missing, present(9)),
    ];
    let original = input(&store, &items);
    let issued = issue_all(&mut store, &original);
    let [(first, first_job), (second, second_job)] = &issued[..] else {
        panic!("夹具应规划两条动作");
    };
    // 第一条执行器当场拿到了回执；第二条停在未知。
    record(&mut store, &original, first_job, &commit(first));
    let mut port = FakePort::new();
    port.recorded = Some(commit(second));
    assert_eq!(run(&mut port, &mut store), counts(1, 0));
    assert_eq!(
        port.replayed,
        vec![second_job.clone()],
        "只问未知原键，用原键原摘要"
    );
    assert_eq!(port.installs, 0, "有原回执时接缝不会走到安装");
    assert!(committed(&status(&store, first)));
    assert!(committed(&status(&store, second)));
    // 再核一次：没有未知，端口一次都不再被问。
    assert_eq!(run(&mut port, &mut store), counts(0, 0));
    assert_eq!(port.replayed.len(), 1);
    assert_eq!(port.probed, 0);
}
