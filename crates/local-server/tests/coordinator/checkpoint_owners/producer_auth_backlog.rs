//! 登录库留着没补记的审计时，封存照样做得成（BUG-044）。
//!
//! 封存冻结登录库时要求它没有待补记的审计事件、没有没交付的拒绝计数；可这些平时只在登录、吊销时补记。
//! 会话过了空闲期、之后没人再登录，那条「会话过期」就一直留在待发件箱里，封存每一轮都失败。
//! 别的库各有周期任务替它清，登录库没有，所以封存自己要在排空之前先把它补完。
use super::{
    fixture::Assembled,
    producer::Stage,
    restore::{owner_identity, wall},
    *,
};
use std::sync::atomic::{AtomicI64, Ordering};
use studiolo_audit_ledger::AuditFilter;
use studiolo_auth::SessionError;
use studiolo_backup::worker::audit::{QueueAnswer, QueueCommand};

/// 配置里的空闲期是 30 分钟（夹具 `owner.json` 的 `idle_seconds`）；拨过它一分钟。
const PAST_IDLE_MILLIS: i64 = 31 * 60 * 1000;

/// 可以往后拨的会话时钟。
struct Dial(AtomicI64);
impl SessionClock for Dial {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        Ok(self.0.load(Ordering::Acquire))
    }
}

/// 登录库里还没补记的：待发件箱里没有回执的事件数、没交付完的拒绝计数类别数。只读，只数行数。
fn backlog(stage: &Stage) -> (i64, i64) {
    let db = rusqlite::Connection::open_with_flags(
        stage.assembled.fixture.root.join("auth/auth.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let count = |sql: &str| db.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();
    (
        count("SELECT count(*) FROM audit_outbox WHERE receipt_json IS NULL"),
        count("SELECT count(*) FROM auth_denials WHERE total!=delivered"),
    )
}

/// 账本里这个动作的记录数。
fn recorded(stage: &Stage, action: &str) -> usize {
    stage
        .assembled
        .human
        .query(&AuditFilter::default().with_action(action))
        .unwrap()
        .len()
}

/// 安排一次封存并等它做完；做成了才会有 worker 的待办。
fn seal(stage: &Stage) -> bool {
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    matches!(
        stage.ask(QueueCommand::NextPending),
        QueueAnswer::Pending(_)
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_that_expired_with_nobody_logging_in_again_does_not_block_the_seal() {
    let clock = Arc::new(Dial(AtomicI64::new(wall())));
    let stage = Stage::assemble(Assembled::with_clock(clock.clone()).await, true, 600_000);
    let sessions = stage.assembled.inputs.sessions.clone();
    let identity = owner_identity().await;
    let issued = sessions.issue(&identity).unwrap();
    assert_eq!(backlog(&stage), (0, 0), "登录这一步会把待发的都补记掉");

    // 过了空闲期再来一次请求：会话被标成过期、事件入队、这次请求被拒。之后没人登录，就没人补记。
    clock.0.fetch_add(PAST_IDLE_MILLIS, Ordering::AcqRel);
    assert!(sessions.check(&identity, &issued.token).is_err());
    let (pending, undelivered) = backlog(&stage);
    assert!(pending >= 1 && undelivered >= 1, "{pending} {undelivered}");
    assert_eq!(recorded(&stage, "session.expired"), 0);

    assert!(
        seal(&stage),
        "登录库留着一条没补记的会话过期事件，封存没做成"
    );
    assert_eq!(backlog(&stage), (0, 0));
    assert_eq!(recorded(&stage, "session.expired"), 1);
    stage.assembled.writable(true);
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_requests_that_were_never_reported_do_not_block_the_seal() {
    let stage = Stage::new(true).await;
    let sessions = stage.assembled.inputs.sessions.clone();
    let identity = owner_identity().await;
    let (first, second) = (
        sessions.issue(&identity).unwrap(),
        sessions.issue(&identity).unwrap(),
    );
    // 拿另一个会话的防跨站令牌来配：被拒，拒绝计数加一；被拒的请求不补记。
    assert!(
        sessions
            .check_with_csrf(&identity, &first.token, &second.csrf)
            .is_err()
    );
    let (_, undelivered) = backlog(&stage);
    assert_eq!(undelivered, 1);
    let before = recorded(&stage, "session.denied");

    assert!(seal(&stage), "登录库留着没交付的拒绝计数，封存没做成");
    assert_eq!(backlog(&stage), (0, 0));
    assert_eq!(recorded(&stage, "session.denied"), before + 1);
    stage.assembled.writable(true);
    stage.assembled.workbench.shutdown().await.unwrap();
}
