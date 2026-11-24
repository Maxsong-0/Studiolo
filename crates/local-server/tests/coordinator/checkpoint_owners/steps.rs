//! 封存分步用时（BUG-050）：窗口里、放开之后各走过哪几步，按顺序一步不漏。没装的库（这里没有
//! Bridge）也照样记一步，日志的形状固定；冻结半路失败时，已走过的步照样留着、没走到的不记。
//!
//! 169 上冻结窗口 26 秒，日志里只有一个总数，分不清是哪一步慢。
use super::{producer::Stage, seal::manifest, *};
use crate::checkpoint::{CheckpointTimes, StepTimes, trusted_catalog};
use studiolo_backup::{
    checkpoint::staging::StagingArea,
    worker::audit::{QueueAnswer, QueueCommand},
};
use studiolo_core::checkpoint::InstantMillis;

/// 冻结窗口里的步骤：补记、等人侧空闲、排空、冻结写者和各库、核 Intent、定格写者两棵树、核 Bridge 回执、
/// 交出定格、Bridge 导出、放开。窗口里不核写者回执（BUG-050）。
pub(super) const WINDOW: [&str; 12] = [
    "补记登录库",
    "等人侧空闲",
    "排空接入方",
    "排空人侧",
    "写者冻结",
    "各库冻结",
    "核 Intent",
    "定格写者两棵树",
    "核 Bridge 回执",
    "交出写者定格",
    "Bridge 导出",
    "放开",
];

/// 放开之后 `seal_timed` 里的步骤；写准入在生产者里另记一步。
pub(super) const AFTER: [&str; 7] = [
    "校验与核回执",
    "写者导出",
    "核写者回执",
    "其余导出",
    "账本导出与验链",
    "登记来源",
    "封存暂存区",
];

pub(super) fn now_millis() -> i64 {
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

/// 人侧有一个完整操作一直不放，冻结等不到人侧空闲、在预算到时失败：前一步记下了，后面的没有，
/// 接入方没被挡过。补记和等人侧空闲共用这一个预算：给 2 秒，测试机满载时补记也走得完；失败本来就要等满预算。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_freeze_that_fails_halfway_keeps_the_steps_it_took() {
    let f = Setup::new(true).await;
    let held = f.human.enter_operation().unwrap();
    let mut steps = StepTimes::start();
    let started = Instant::now();
    let result = f.owners.freeze_timed(
        f.request(2_000),
        &f.key,
        BackupTargetKind::LocalTemporaryRepository,
        f.targets(),
        &mut steps,
    );
    drop(held);
    assert_eq!(
        result.err(),
        Some(CheckpointError::NotDrained {
            owner: Owner::Audit
        }),
        "人侧空不下来，冻结不该成"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "等过了冻结预算：{:?}",
        started.elapsed()
    );
    assert_eq!(steps.names(), ["补记登录库"]);
    f.workbench.shutdown().await.unwrap();
}

/// 经生产者走一次完整封存：窗口那一行、放开之后那一行带的分步，和直接调 `CheckpointOwners` 记下的一样，
/// 写准入记在放开之后的最后一步。生产者要是换了一份新计时、漏了一步，这里就红。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_producer_logs_every_step_on_both_lines() {
    let stage = Stage::new(true).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    let lines = stage.producer.seal_lines.lock().unwrap().clone();
    assert_eq!(lines.len(), 2, "封存要成功、打出两行：{lines:?}");
    assert_eq!(logged_steps(&lines[0], "备份封存冻结窗口："), WINDOW);
    let mut after = AFTER.to_vec();
    after.push("写准入");
    assert_eq!(logged_steps(&lines[1], "备份封存放开之后："), after);
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 日志行「前缀N 毫秒（分步：名 毫秒、名 毫秒）」里的步名，按顺序。
fn logged_steps<'a>(line: &'a str, prefix: &str) -> Vec<&'a str> {
    assert!(line.starts_with(prefix), "{line}");
    let (_, steps) = line.split_once("（分步：").expect(line);
    let steps = steps.strip_suffix('）').expect(line);
    steps
        .split('、')
        .map(|step| step.rsplit_once(' ').expect(step).0)
        .collect()
}
