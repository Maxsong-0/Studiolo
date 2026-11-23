//! 封存分步用时（BUG-050）：窗口里、放开之后各走过哪几步，按顺序一步不漏。没装的库（这里没有
//! Bridge）也照样记一步，日志的形状固定；冻结半路失败时，已走过的步照样留着、没走到的不记。
//!
//! 169 上冻结窗口 26 秒，日志里只有一个总数，分不清是哪一步慢。
use super::{seal::manifest, *};
use crate::checkpoint::{CheckpointTimes, StepTimes, trusted_catalog};
use studiolo_backup::checkpoint::staging::StagingArea;
use studiolo_core::checkpoint::InstantMillis;

/// 冻结窗口里的步骤：补记、排空、定格、冻结、窗口里核的三类回执、交出定格、Bridge 导出、放开。
const WINDOW: [&str; 11] = [
    "补记登录库",
    "排空接入方",
    "排空人侧",
    "写者定格",
    "各库冻结",
    "核 Intent",
    "核写者回执",
    "核 Bridge 回执",
    "交出写者定格",
    "Bridge 导出",
    "放开",
];

/// 放开之后 `seal_timed` 里的步骤；写准入在生产者里另记一步。
const AFTER: [&str; 6] = [
    "校验与核回执",
    "写者导出",
    "其余导出",
    "账本导出与验链",
    "登记来源",
    "封存暂存区",
];

fn now_millis() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_records_every_step_in_order() {
    let f = Setup::new(true).await;
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
    let frozen = f
        .owners
        .freeze_timed(
            f.request(30_000),
            &f.key,
            BackupTargetKind::LocalTemporaryRepository,
            OwnerExportPaths::under(area.export_root()),
            &mut steps,
        )
        .unwrap();
    let mut released = frozen
        .release_timed(Instant::now() + Duration::from_secs(30), &mut steps)
        .unwrap();
    assert_eq!(steps.names(), WINDOW, "窗口里的分步漏了或顺序不对");
    let now = now_millis();
    let mut after = StepTimes::start();
    let sealed = released.seal_timed(
        area,
        trusted_catalog,
        manifest(),
        CheckpointTimes {
            content_cutoff: InstantMillis::from_millis(now - 100),
            now: InstantMillis::from_millis(now),
            expires_at: InstantMillis::from_millis(now + 30_000),
        },
        &mut after,
    );
    assert!(sealed.is_ok(), "封存没成：{:?}", sealed.err());
    assert_eq!(after.names(), AFTER, "放开之后的分步漏了或顺序不对");
    drop(released);
    f.workbench.shutdown().await.unwrap();
}

/// 人侧有一个完整操作一直不放，冻结排不空人侧、在预算到时失败：前两步记下了，后面的没有。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_freeze_that_fails_halfway_keeps_the_steps_it_took() {
    let f = Setup::new(true).await;
    let held = f.human.enter_operation().unwrap();
    let mut steps = StepTimes::start();
    let result = f.owners.freeze_timed(
        f.request(300),
        &f.key,
        BackupTargetKind::LocalTemporaryRepository,
        f.targets(),
        &mut steps,
    );
    drop(held);
    assert!(result.is_err(), "人侧排不空，冻结不该成");
    drop(result);
    assert_eq!(steps.names(), ["补记登录库", "排空接入方"]);
    f.workbench.shutdown().await.unwrap();
}
