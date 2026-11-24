//! 封存先等人侧空下来再冻结（BUG-056）：人侧有完整操作做得比冻结预算还久（外网不通时的一轮同步），
//! 封存安静地等它做完再封；等的时候人侧、接入方都不受影响，等的时间不算进冻结预算和封存总预算。
//!
//! 169 上 2026-11-24 19:55Z：一轮同步 61 秒才退出，封存排空人侧等满 30 秒就放弃，少一份恢复点；
//! 那 30 秒里接入方账本已经被封存独占，AI 侧陪着等。
use super::{producer::Stage, *};
use crate::{
    backup::producer::BackupProducer,
    checkpoint::{HumanSide, StepTimes},
};
use std::{sync::mpsc, thread};
use studiolo_backup::worker::audit::{QueueAnswer, QueueCommand};

/// 慢操作占着的时长：比给的冻结预算、总预算都久。
const SLOW: Duration = Duration::from_secs(8);
/// 给的总预算（正文保留时长）和冻结预算，毫秒。
const RETENTION: u64 = 7_000;
const FREEZE: u32 = 3_000;

/// `span` 这段时间里隔一会儿试一次完整操作许可，回有几次拿不到；拿到当场还掉。
fn refused(ledger: &SqliteAuditLedger, span: Duration) -> usize {
    let until = Instant::now() + span;
    let mut refused = 0;
    while Instant::now() < until {
        refused += usize::from(ledger.try_enter_operation().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    refused
}

/// 等一件会等别人的事做完，最多等 `span`；回做完没有。没做完的那个线程留着，跟着用例一起收。
fn within(
    span: Duration,
    producer: &Arc<BackupProducer>,
    wait: impl FnOnce(&BackupProducer) + Send + 'static,
) -> bool {
    let producer = producer.clone();
    let (finished, done) = mpsc::channel();
    thread::spawn(move || {
        wait(&producer);
        let _ = finished.send(());
    });
    done.recv_timeout(span).is_ok()
}

/// 日志行「前缀N 毫秒…」里的毫秒数。
fn millis(line: &str, prefix: &str) -> u128 {
    let rest = line.strip_prefix(prefix).expect(line);
    rest.split_once(" 毫秒").expect(line).0.parse().expect(line)
}

/// 慢操作做完之后封存照样成功；等的时候两侧的许可随时拿得到；等的时间另打一行、不算进窗口。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_outwaits_a_slow_human_operation_without_blocking_anyone() {
    let stage = Stage::with_budgets(RETENTION, FREEZE).await;
    let slow = stage.assembled.human.enter_operation().unwrap();
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    let (agents, humans) = thread::scope(|scope| {
        let humans = scope.spawn(|| refused(&stage.assembled.human, SLOW));
        (
            refused(&stage.assembled.agent, SLOW),
            humans.join().unwrap(),
        )
    });
    drop(slow);
    stage.producer.settle();
    assert_eq!(
        (agents, humans),
        (0, 0),
        "等人侧空闲的时候，接入方、人侧的完整操作都不该被挡"
    );
    assert!(
        matches!(
            stage.ask(QueueCommand::NextPending),
            QueueAnswer::Pending(_)
        ),
        "慢操作做完之后封存要成功"
    );
    let lines = stage.producer.seal_lines.lock().unwrap().clone();
    assert_eq!(lines.len(), 3, "等的那行、窗口、放开之后：{lines:?}");
    let waited = millis(&lines[0], "备份封存先等人侧空闲：");
    let window = millis(&lines[1], "备份封存冻结窗口：");
    assert!(
        waited > u128::from(FREEZE),
        "等了 {waited} 毫秒，没等过冻结预算"
    );
    assert!(
        waited < (SLOW + Duration::from_secs(5)).as_millis(),
        "慢操作做完好一会儿才发现：等了 {waited} 毫秒"
    );
    assert!(
        window < waited,
        "等的 {waited} 毫秒算进了窗口：{window} 毫秒"
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 冻结自己开头也先等人侧空下来，再去挡接入方：补记登录库和排空之间新起的慢操作，不会让接入方陪着等。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_freeze_waits_for_the_human_side_before_it_blocks_the_agents() {
    let f = Setup::new(true).await;
    let slow = f.human.enter_operation().unwrap();
    let mut steps = StepTimes::start();
    let (agents, frozen) = thread::scope(|scope| {
        let agents = scope.spawn(|| {
            let refused = refused(&f.agent, Duration::from_millis(1_500));
            drop(slow);
            refused
        });
        let frozen = f
            .owners
            .freeze_timed(
                f.request(20_000),
                &f.key,
                BackupTargetKind::LocalTemporaryRepository,
                f.targets(),
                &mut steps,
            )
            .map(drop);
        (agents.join().unwrap(), frozen)
    });
    assert_eq!(frozen, Ok(()), "慢操作在预算之内做完，冻结要成");
    assert_eq!(agents, 0, "人侧还没空下来，不该先挡接入方");
    assert_eq!(
        steps.names()[..4],
        ["补记登录库", "等人侧空闲", "排空接入方", "排空人侧"]
    );
    f.workbench.shutdown().await.unwrap();
}

/// 关停时封存还在等人侧空闲：马上放弃、补失败结果，不等慢操作做完；慢操作做完之后也不再冻结。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_does_not_wait_for_a_seal_that_is_still_waiting() {
    let stage = Stage::new(true).await;
    let slow = stage.assembled.human.enter_operation().unwrap();
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    // 让它走到等待里；没走到也一样，走到时第一眼就看见在关停。
    thread::sleep(Duration::from_millis(300));
    let started = Instant::now();
    // 上限给得比 1 秒宽：测试机满载也够，和不看关停时的几分钟分得开；实际用时打出来看。
    let returned = within(Duration::from_secs(10), &stage.producer, |producer| {
        producer.settle_window();
    });
    println!("关停等了 {} 毫秒", started.elapsed().as_millis());
    assert!(returned, "封存还在等人侧空闲，关停要能马上返回");
    assert_eq!(stage.outcomes(1), 1, "放弃的这次有失败结果");
    drop(slow);
    stage.producer.settle();
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(
        *stage.producer.seal_lines.lock().unwrap(),
        Vec::<String>::new(),
        "关停之后不该再冻结"
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 关停已经开始，之后才轮到的封存也不再冻结，人侧空着也一样。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_that_starts_after_shutdown_began_never_freezes() {
    let stage = Stage::new(true).await;
    stage.producer.settle_window();
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    assert_eq!(stage.outcomes(1), 1, "放弃的这次有失败结果");
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(
        *stage.producer.seal_lines.lock().unwrap(),
        Vec::<String>::new()
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 等到了人侧空闲、还没开始冻结的时候来的关停：同样不再开始，关停不等一整个冻结窗口。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_told_to_stop_between_the_wait_and_the_freeze_never_freezes() {
    let stage = Stage::new(true).await;
    let (open, gate) = mpsc::channel();
    *stage.producer.hold_before_freeze.lock().unwrap() = Some(gate);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    // 封存走到这里才把闸拿走。
    let begun = Instant::now();
    while stage.producer.hold_before_freeze.lock().unwrap().is_some() {
        assert!(
            begun.elapsed() < Duration::from_secs(60),
            "封存没走到冻结之前"
        );
        thread::sleep(Duration::from_millis(2));
    }
    let producer = stage.producer.clone();
    let (settled, window) = mpsc::channel();
    thread::spawn(move || {
        producer.settle_window();
        let _ = settled.send(());
    });
    assert!(
        window.recv_timeout(Duration::from_millis(500)).is_err(),
        "封存线程还停着，关停要等它"
    );
    open.send(()).unwrap();
    assert!(window.recv_timeout(Duration::from_secs(60)).is_ok());
    stage.producer.settle();
    assert_eq!(stage.outcomes(1), 1, "放弃的这次有失败结果");
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(
        *stage.producer.seal_lines.lock().unwrap(),
        Vec::<String>::new(),
        "关停叫停之后不该再冻结"
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 等有上限：到上限这次封存照旧失败、补失败结果、占满间隔；一路上接入方没被挡过，什么都没冻。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_gives_up_when_the_human_side_never_goes_idle() {
    let stage = Stage::new(true).await;
    *stage.producer.idle_budget.lock().unwrap() = Duration::from_millis(2_500);
    let slow = stage.assembled.human.enter_operation().unwrap();
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    let agents = refused(&stage.assembled.agent, Duration::from_secs(1));
    assert_eq!(stage.outcomes(1), 0, "没等满上限就放弃了");
    let agents = agents + refused(&stage.assembled.agent, Duration::from_secs(2));
    assert!(
        within(Duration::from_secs(20), &stage.producer, |producer| {
            producer.settle();
        }),
        "到上限要放弃，不能一直等"
    );
    assert_eq!(agents, 0, "等不到就放弃，接入方一路都不该被挡");
    assert_eq!(stage.outcomes(1), 1);
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(
        *stage.producer.seal_lines.lock().unwrap(),
        Vec::<String>::new()
    );
    drop(slow);
    stage.assembled.writable(true);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::NotDue);
    stage.producer.settle();
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 等待本身：叫停先于看，看到空就回，等到上限报没排空，审计库不健康当场报不可用、不当成在忙。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn waiting_for_the_human_side_stops_when_told_to_and_fails_on_an_unhealthy_ledger() {
    let f = Setup::new(true).await;
    let far = || Instant::now() + Duration::from_secs(60);
    // 等过了头就叫停：上限没守住时这里报错，不把用例挂住。
    let until = far();
    let nap = |span| {
        thread::sleep(span);
        Instant::now() >= until
    };
    assert_eq!(f.owners.await_human_idle(far(), nap), Ok(HumanSide::Idle));
    assert_eq!(
        f.owners.await_human_idle(far(), |_| true),
        Ok(HumanSide::Stopped),
        "叫停了就不再看，人侧空着也一样"
    );
    let slow = f.human.enter_operation().unwrap();
    assert_eq!(
        f.owners
            .await_human_idle(Instant::now() + Duration::from_millis(200), nap),
        Err(CheckpointError::NotDrained {
            owner: Owner::Audit
        })
    );
    drop(slow);
    let audit = f.fixture.root.join("audit");
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o750)).unwrap();
    let started = Instant::now();
    let unhealthy = f.owners.await_human_idle(far(), nap);
    let waited = started.elapsed();
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        unhealthy,
        Err(CheckpointError::OwnerUnavailable {
            owner: Owner::Audit
        })
    );
    assert!(waited < Duration::from_secs(10), "不健康不是在忙，不该等");
    f.workbench.shutdown().await.unwrap();
}

/// 排空等到截止的报「在预算内未排空」，账本本身不健康的当场报「不可用」（以前都报「不可用」）；走过的步照样留着。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drain_that_runs_out_of_budget_says_so() {
    let f = Setup::new(true).await;
    let held = f.agent.enter_operation().unwrap();
    assert_eq!(
        undrained(&f, 1_500),
        (
            CheckpointError::NotDrained {
                owner: Owner::AgentStore
            },
            vec!["补记登录库", "等人侧空闲"]
        )
    );
    drop(held);
    f.workbench.shutdown().await.unwrap();

    let f = Setup::new(true).await;
    fs::set_permissions(
        f.fixture.root.join("agent/audit"),
        fs::Permissions::from_mode(0o750),
    )
    .unwrap();
    let started = Instant::now();
    assert_eq!(
        undrained(&f, 30_000),
        (
            CheckpointError::OwnerUnavailable {
                owner: Owner::AgentStore
            },
            vec!["补记登录库", "等人侧空闲"]
        )
    );
    assert!(started.elapsed() < Duration::from_secs(20), "不健康不该等");
    f.workbench.shutdown().await.unwrap();

    // 看过人侧空闲之后、排空人侧之前，人侧又起了一个一直不放的操作：接入方先排着，正好给它插进来的空。
    let f = Setup::new(true).await;
    let agent = f.agent.enter_operation().unwrap();
    let (finish, finished) = mpsc::channel::<()>();
    let human = f.human.clone();
    let outcome = thread::scope(|scope| {
        scope.spawn(move || {
            thread::sleep(Duration::from_millis(1_500));
            let human = human.enter_operation().unwrap();
            drop(agent);
            let _ = finished.recv();
            drop(human);
        });
        let outcome = undrained(&f, 5_000);
        finish.send(()).unwrap();
        outcome
    });
    assert_eq!(
        outcome,
        (
            CheckpointError::NotDrained {
                owner: Owner::Audit
            },
            vec!["补记登录库", "等人侧空闲", "排空接入方"]
        )
    );
    f.workbench.shutdown().await.unwrap();
}

/// 给这么多预算去冻结，一定冻不成：回是哪一类、走过哪几步。
fn undrained(f: &Setup, millis: u32) -> (CheckpointError, Vec<&'static str>) {
    let mut steps = StepTimes::start();
    let failure = f
        .owners
        .freeze_timed(
            f.request(millis),
            &f.key,
            BackupTargetKind::LocalTemporaryRepository,
            f.targets(),
            &mut steps,
        )
        .map(drop)
        .expect_err("排不空，冻结不该成");
    (failure, steps.names())
}
