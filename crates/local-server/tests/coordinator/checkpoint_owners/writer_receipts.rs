//! 写者的逐条回执在放开之后、从导出的副本上核（BUG-050）：冻结窗口里不读回执，核不上照样不出票。
//!
//! 169 上窗口 11.6 秒，其中 11.3 秒在逐条核写者回执；回执随编辑数涨，窗口也跟着涨。
use super::{
    fixture::Assembled,
    producer::Stage,
    seal::manifest,
    steps::{AFTER, WINDOW, now_millis},
    *,
};
use crate::checkpoint::{CheckpointTimes, StepTimes, trusted_catalog};
use studiolo_backup::{
    checkpoint::staging::StagingArea,
    worker::audit::{QueueAnswer, QueueCommand},
};
use studiolo_core::checkpoint::InstantMillis;

/// 回执和账本对不上（同一事件号换了动作；回执记的哈希不对）：冻结、放开照常走完，写请求不被挡；
/// 封存在放开之后停在「核写者回执」，走过的步留着，没有封存结果。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_receipt_that_disagrees_with_the_ledger_fails_the_seal_after_the_window() {
    for corrupt in [1, 2] {
        let f = Setup::of(Assembled::with_bad_receipt(corrupt).await, true);
        let sources = [
            "vault", "data", "auth", "control", "agent", "policy", "audit",
        ]
        .map(|name| f.fixture.root.join(name));
        let area = StagingArea::create(
            &target(&f.fixture, "staging"),
            f.key.checkpoint_id.clone(),
            &sources,
            &target(&f.fixture, "ephemeral"),
            Instant::now() + Duration::from_secs(25),
        )
        .unwrap();
        let mut steps = StepTimes::start();
        let frozen = f.owners.freeze_timed(
            f.request(30_000),
            &f.key,
            BackupTargetKind::LocalTemporaryRepository,
            OwnerExportPaths::under(area.export_root()),
            &mut steps,
        );
        let mut released = frozen
            .unwrap_or_else(|error| panic!("回执坏了（{corrupt}）不该在窗口里失败：{error}"))
            .release_timed(Instant::now() + Duration::from_secs(30), &mut steps)
            .unwrap();
        assert_eq!(steps.names(), WINDOW);
        f.writable(true);
        assert!(f.human.enter_operation().is_ok());
        assert!(f.workbench.core.owner.lock().unwrap().is_some());
        let now = now_millis();
        let mut after = StepTimes::start();
        let failure = released
            .seal_timed(
                area,
                trusted_catalog,
                manifest(),
                CheckpointTimes {
                    content_cutoff: InstantMillis::from_millis(now - 100),
                    now: InstantMillis::from_millis(now),
                    expires_at: InstantMillis::from_millis(now + 30_000),
                },
                &mut after,
            )
            .err()
            .unwrap_or_else(|| panic!("回执坏了（{corrupt}）不该封得成"));
        assert_eq!(
            failure.to_string(),
            "核写者回执：owner Broker 不可用",
            "{corrupt}"
        );
        assert_eq!(after.names(), AFTER[..2], "{corrupt}");
        drop(released);
        f.workbench.shutdown().await.unwrap();
    }
}

/// 同一件事走一遍生产者：「备份封存冻结窗口」那行照打，没有「放开之后」做成的那一行；
/// 原键记一条失败结果，worker 没有待办，业务照常读写。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_producer_still_logs_the_window_and_records_the_failure() {
    let stage = Stage::assemble(Assembled::with_bad_receipt(1).await, true, 600_000);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    let lines = stage.producer.seal_lines.lock().unwrap().clone();
    assert_eq!(
        lines.len(),
        2,
        "窗口照常走完打一行，放开之后卡在哪打一行，没有做成的那行：{lines:?}"
    );
    assert!(lines[0].starts_with("备份封存冻结窗口："), "{lines:?}");
    assert!(
        lines[1].starts_with(
            "备份封存放开之后卡在：核写者回执：owner Broker 不可用（已走：校验与核回执 "
        ),
        "{lines:?}"
    );
    stage.assembled.writable(true);
    assert!(stage.assembled.human.enter_operation().is_ok());
    assert_eq!(stage.outcomes(1), 1, "没做成要记一条失败结果");
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    stage.assembled.workbench.shutdown().await.unwrap();
}
