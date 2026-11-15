//! 出站门铃走真实文件与真实账本的一趟：状态存储的失败关闭，以及
//! SQLite 账本 ＋ 私有状态目录 ＋ 主循环连起来的行为（ADR-033）。

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Mutex,
};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
    SqliteAuditLedger,
};
use studiolo_observability::{
    doorbell::{
        DOORBELL_AUDIT_ACTION, DoorbellState, DoorbellTransport, PostError, RunError, RunOutcome,
        run,
    },
    store::{DOORBELL_STAGE_FILE_NAME, DOORBELL_STATE_FILE_NAME, DoorbellStore, StoreError},
};

struct Fixture {
    _directory: tempfile::TempDir,
    audit: PathBuf,
    state: PathBuf,
}

fn private(path: &Path) {
    fs::create_dir(path).expect("夹具目录必须可创建");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("私有目录权限必须可设置");
}

fn fixture() -> Fixture {
    let directory = tempfile::TempDir::new().expect("临时目录必须可创建");
    let root = fs::canonicalize(directory.path()).expect("临时目录必须可规范化");
    let (audit, state) = (root.join("audit"), root.join("doorbell"));
    private(&audit);
    private(&state);
    Fixture {
        _directory: directory,
        audit,
        state,
    }
}

fn sample() -> DoorbellState {
    DoorbellState {
        cursor: 12,
        attempts: 1,
        retry_at_ms: 1_700_000_000_000,
        gone: false,
        endpoint_tag: "0123456789abcdef".into(),
    }
}

fn names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(root)
        .expect("目录必须可枚举")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// 记下收到的消息编号；回执一律收下。
struct Recorder(Mutex<Vec<String>>);

impl DoorbellTransport for Recorder {
    fn endpoint_tag(&self) -> Result<String, PostError> {
        Ok("00000000000000aa".into())
    }

    fn link(&self) -> Option<&str> {
        None
    }

    fn post(&self, message_id: &str, _: i64, _: &[u8]) -> Result<(), PostError> {
        self.0.lock().unwrap().push(message_id.to_owned());
        Ok(())
    }
}

fn now() -> i64 {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("时钟在 1970 年之后");
    i64::try_from(elapsed.as_millis()).expect("毫秒时间戳装得下")
}

fn task(ledger: &SqliteAuditLedger, n: u32) {
    ledger
        .append(AuditEvent::new(
            AuditEventId::parse(format!("board:task.create:{n}")).unwrap(),
            AuditPhase::Outcome,
            "task.create",
            SanitizedPayload::try_from(
                serde_json::json!({"request_hash": n.to_string(), "status": "succeeded"}),
            )
            .unwrap(),
        ))
        .unwrap();
}

#[test]
fn 状态只落在自己的固定文件名上且读回一致() {
    let fixture = fixture();
    let store = DoorbellStore::open(&fixture.state).expect("私有目录必须可绑定");
    assert_eq!(store.load(), Ok(None));
    store.save(&sample()).expect("保存必须成功");
    assert_eq!(store.load(), Ok(Some(sample())));
    assert_eq!(names(&fixture.state), [DOORBELL_STATE_FILE_NAME]);
    let mode = fs::metadata(fixture.state.join(DOORBELL_STATE_FILE_NAME))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o600);
}

#[test]
fn 撕裂的状态和不私有的目录都失败关闭() {
    let fixture = fixture();
    let store = DoorbellStore::open(&fixture.state).expect("私有目录必须可绑定");
    store.save(&sample()).expect("保存必须成功");
    let path = fixture.state.join(DOORBELL_STATE_FILE_NAME);
    let mut bytes = fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 2);
    fs::write(&path, &bytes).unwrap();
    assert_eq!(store.load(), Err(StoreError::Corrupt));

    // 上一轮崩在中途留下的暂存件不影响下一次保存，也不会被当成状态。
    let stage = fixture.state.join(DOORBELL_STAGE_FILE_NAME);
    fs::write(&stage, b"half").unwrap();
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o600)).unwrap();
    store.save(&sample()).expect("保存必须成功");
    assert_eq!(names(&fixture.state), [DOORBELL_STATE_FILE_NAME]);
    assert_eq!(store.load(), Ok(Some(sample())));

    fs::set_permissions(&fixture.state, fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(store.load(), Err(StoreError::UnsafeStorage));
    assert_eq!(store.save(&sample()), Err(StoreError::UnsafeStorage));
    assert!(DoorbellStore::open(&fixture.state).is_err());
}

#[test]
fn 真实账本和状态目录连起来按位置往后发且重开后接着发() {
    let fixture = fixture();
    let recorder = Recorder(Mutex::new(Vec::new()));
    {
        let ledger = SqliteAuditLedger::open(&fixture.audit).expect("账本必须可初始化");
        let store = DoorbellStore::open(&fixture.state).expect("私有目录必须可绑定");
        task(&ledger, 1);
        let enabled = run(&ledger, &store, &recorder, now()).unwrap();
        assert_eq!(enabled.outcome, RunOutcome::Enabled);
        task(&ledger, 2);
        task(&ledger, 3);
        let summary = run(&ledger, &store, &recorder, now()).unwrap();
        assert_eq!(
            (summary.outcome, summary.delivered),
            (RunOutcome::CaughtUp, 2)
        );
    }
    // 进程重启：只开已有的账本和状态，从上次的位置接着来。
    let ledger = SqliteAuditLedger::open_existing(&fixture.audit).expect("账本必须可重开");
    let store = DoorbellStore::open(&fixture.state).expect("私有目录必须可绑定");
    let summary = run(&ledger, &store, &recorder, now()).unwrap();
    assert_eq!(
        (summary.outcome, summary.delivered),
        (RunOutcome::CaughtUp, 0)
    );
    task(&ledger, 4);
    let summary = run(&ledger, &store, &recorder, now()).unwrap();
    assert_eq!(
        (summary.outcome, summary.delivered),
        (RunOutcome::CaughtUp, 1)
    );
    let posted = recorder.0.lock().unwrap().clone();
    assert_eq!(posted.len(), 3);
    let unique: std::collections::BTreeSet<&String> = posted.iter().collect();
    assert_eq!(unique.len(), 3);
    // 每条出门的事件在权威账本里各有一条意图、一条结果。
    let audited = ledger
        .query(&AuditFilter::default().with_action(DOORBELL_AUDIT_ACTION))
        .unwrap();
    assert_eq!(audited.len(), 6);

    // 状态被删掉不等于「没启用过」的另一面：状态坏了要报错，不许悄悄从头来。
    fs::write(fixture.state.join(DOORBELL_STATE_FILE_NAME), b"{}").unwrap();
    assert_eq!(
        run(&ledger, &store, &recorder, now()),
        Err(RunError::StateUnavailable)
    );
    assert_eq!(recorder.0.lock().unwrap().len(), 3);
}

/// 发送时去试一次备份封存（独占排空），记下结果。
struct FreezeProbe<'a> {
    ledger: &'a SqliteAuditLedger,
    frozen_during_post: Mutex<Vec<bool>>,
}

impl DoorbellTransport for FreezeProbe<'_> {
    fn endpoint_tag(&self) -> Result<String, PostError> {
        Ok("00000000000000aa".into())
    }

    fn link(&self) -> Option<&str> {
        None
    }

    fn post(&self, _: &str, _: i64, _: &[u8]) -> Result<(), PostError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        let frozen = self.ledger.quiesce_operations(deadline).is_ok();
        self.frozen_during_post.lock().unwrap().push(frozen);
        Ok(())
    }
}

#[test]
fn 发送途中备份封存排不空而封存期间一条都不发() {
    let fixture = fixture();
    let ledger = SqliteAuditLedger::open(&fixture.audit).expect("账本必须可创建");
    let store = DoorbellStore::open(&fixture.state).expect("私有目录必须可绑定");
    let probe = FreezeProbe {
        ledger: &ledger,
        frozen_during_post: Mutex::new(Vec::new()),
    };
    assert_eq!(
        run(&ledger, &store, &probe, now()).unwrap().outcome,
        RunOutcome::Enabled
    );
    task(&ledger, 1);
    assert_eq!(run(&ledger, &store, &probe, now()).unwrap().delivered, 1);
    // 意图已记、请求在途时，这条事件还握着操作许可：封存必须等它做完。
    assert_eq!(*probe.frozen_during_post.lock().unwrap(), [false]);

    // 反过来：封存占着的时候，门铃取不到许可，整条不开始。
    task(&ledger, 2);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let frozen = ledger
        .quiesce_operations(deadline)
        .expect("空闲时封存必须成功");
    let recorder = Recorder(Mutex::new(Vec::new()));
    assert_eq!(
        run(&ledger, &store, &recorder, now()),
        Err(RunError::AuditUnavailable)
    );
    assert!(recorder.0.lock().unwrap().is_empty());
    drop(frozen);
    assert_eq!(run(&ledger, &store, &recorder, now()).unwrap().delivered, 1);
}

#[test]
fn 同一个状态目录同一时间只让一个门铃进程打开() {
    let fixture = fixture();
    let first = DoorbellStore::open(&fixture.state).expect("私有目录必须可绑定");
    first.save(&sample()).expect("保存必须成功");
    // 第二个打开者（另一个进程，或同进程的另一个句柄）直接让开，不等待、不读不写。
    assert_eq!(
        DoorbellStore::open(&fixture.state).err(),
        Some(StoreError::Busy)
    );
    first.save(&sample()).expect("持有者照常保存");
    drop(first);
    let second = DoorbellStore::open(&fixture.state).expect("放下之后可以再打开");
    assert_eq!(second.load(), Ok(Some(sample())));
}
