//! 死人开关持久存储的行为／失败／恢复三类契约。
//!
//! 这是本 crate 唯一做真实 IO 的边界，所有失败一律**失败关闭**：
//! 半写、校验和不符、未知版本、未知字段、超限都返回错误，
//! 绝不降级成"当作没有状态"，否则一次撕裂写入就会被洗成"刚刚重置"。

mod support;

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_observability::alert::{
    AlertDispatcher, DeadmanDecision, DeadmanPolicy, DeadmanState, evaluate,
};
use studiolo_observability::store::{DeadmanStore, STAGE_FILE_NAME, STATE_FILE_NAME, StoreError};
use support::ScriptedTransport;

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
}

fn fixture() -> Fixture {
    let directory = tempfile::TempDir::new().expect("临时目录必须可创建");
    let root = fs::canonicalize(directory.path()).expect("临时目录必须可规范化");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("私有目录权限必须可设置");
    Fixture {
        _directory: directory,
        root,
    }
}

fn state() -> DeadmanState {
    DeadmanState {
        last_receipt: Some(1_700),
        last_observed: 1_900,
        last_alert: Some(1_800),
        suppressed_since_alert: 2,
        episode: 7,
        dispatch_round: 9,
    }
}

fn write_state_file(root: &std::path::Path, bytes: &[u8]) {
    let path = root.join(STATE_FILE_NAME);
    fs::write(&path, bytes).expect("夹具写入必须成功");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("夹具权限必须可设置");
}

#[test]
fn 首次运行时固定文件不存在于是读到空状态() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");

    assert_eq!(store.load(), Ok(None));
}

#[test]
fn 保存后再读取得到完全相同的状态() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");

    store.save(&state()).expect("保存必须成功");

    assert_eq!(store.load(), Ok(Some(state())));
}

#[test]
fn 保存只落在固定文件名上并且不留下暂存件() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");

    store.save(&state()).expect("保存必须成功");

    let names = fs::read_dir(&fixture.root)
        .expect("目录必须可枚举")
        .map(|entry| entry.expect("目录项必须可读").file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![std::ffi::OsString::from(STATE_FILE_NAME)]);
}

#[test]
fn 连续保存完整替换而不是追加() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    store.save(&state()).expect("第一次保存必须成功");

    let mut next = state();
    next.episode = 8;
    next.dispatch_round = 10;
    store.save(&next).expect("第二次保存必须成功");

    assert_eq!(store.load(), Ok(Some(next)));
}

#[test]
fn 半写的状态文件一律失败关闭而不是当作空状态() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    store.save(&state()).expect("保存必须成功");
    let full = fs::read(fixture.root.join(STATE_FILE_NAME)).expect("状态文件必须可读");

    write_state_file(&fixture.root, &full[..full.len() / 2]);

    assert_eq!(store.load(), Err(StoreError::Corrupt));
}

#[test]
fn 空文件也算半写而不是当作没有状态() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");

    write_state_file(&fixture.root, b"");

    assert_eq!(store.load(), Err(StoreError::Corrupt));
}

#[test]
fn 字段被改动而校验和未同步时判定为损坏() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    store.save(&state()).expect("保存必须成功");
    let text = fs::read_to_string(fixture.root.join(STATE_FILE_NAME)).expect("状态文件必须可读");

    write_state_file(
        &fixture.root,
        text.replace("\"episode\":7", "\"episode\":8").as_bytes(),
    );

    assert_eq!(store.load(), Err(StoreError::Corrupt));
}

#[test]
fn 版本未知时拒绝读取而不是自动迁移或重置() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    store.save(&state()).expect("保存必须成功");
    let text = fs::read_to_string(fixture.root.join(STATE_FILE_NAME)).expect("状态文件必须可读");

    write_state_file(
        &fixture.root,
        text.replace("\"version\":1", "\"version\":2").as_bytes(),
    );

    assert_eq!(store.load(), Err(StoreError::UnsupportedSchema));
}

#[test]
fn 出现未知字段时拒绝读取() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    store.save(&state()).expect("保存必须成功");
    let text = fs::read_to_string(fixture.root.join(STATE_FILE_NAME)).expect("状态文件必须可读");

    write_state_file(
        &fixture.root,
        text.replace("{\"version\":1", "{\"extra\":true,\"version\":1")
            .as_bytes(),
    );

    assert_eq!(store.load(), Err(StoreError::Corrupt));
}

#[test]
fn 状态文件超过字节预算时拒绝读取() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");

    write_state_file(&fixture.root, &vec![b' '; 8 * 1024 + 1]);

    assert_eq!(store.load(), Err(StoreError::TooLarge));
}

#[test]
fn 私有目录权限不是七零零时拒绝绑定() {
    let fixture = fixture();
    fs::set_permissions(&fixture.root, fs::Permissions::from_mode(0o755))
        .expect("夹具权限必须可设置");

    assert_eq!(
        DeadmanStore::open(&fixture.root).err(),
        Some(StoreError::UnsafeStorage)
    );
}

#[test]
fn 目录不存在时拒绝绑定而不是创建目录() {
    let fixture = fixture();
    let missing = fixture.root.join("not-created");

    assert_eq!(
        DeadmanStore::open(&missing).err(),
        Some(StoreError::Missing)
    );
    assert!(!missing.exists());
}

#[test]
fn 相对路径根一律拒绝() {
    assert_eq!(
        DeadmanStore::open(std::path::Path::new("relative")).err(),
        Some(StoreError::UnsafeStorage)
    );
}

#[test]
fn 状态文件权限不是六零零时拒绝读取() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    store.save(&state()).expect("保存必须成功");
    fs::set_permissions(
        fixture.root.join(STATE_FILE_NAME),
        fs::Permissions::from_mode(0o644),
    )
    .expect("夹具权限必须可设置");

    assert_eq!(store.load(), Err(StoreError::UnsafeStorage));
}

#[test]
fn 上一轮崩在中途留下的暂存件会在下次保存时被清掉() {
    let fixture = fixture();
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    let stage = fixture.root.join(STAGE_FILE_NAME);
    fs::write(&stage, b"half written").expect("夹具写入必须成功");

    store.save(&state()).expect("保存必须成功");

    assert!(!stage.exists());
    assert_eq!(store.load(), Ok(Some(state())));
}

#[test]
fn 死人开关状态跨重启恢复后情节与调度轮次都保持不变() {
    let fixture = fixture();
    let policy = DeadmanPolicy::new(600, 60, 3_600, 3).expect("固定测试参数必须合法");
    let before = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    let start = DeadmanState::started_at(1_000).with_receipt(1_000);
    let (fired, decision) = evaluate(policy, start, 1_700).expect("固定正文必须可构造");
    let DeadmanDecision::Fire { alert: first, .. } = decision else {
        panic!("超期必须触发");
    };
    before.save(&fired).expect("保存必须成功");

    // 进程重启：换一个 store 实例，只从固定文件恢复。
    let after = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    let restored = after.load().expect("读取必须成功").expect("必须有状态");

    assert_eq!(restored, fired);
    let (_, decision) = evaluate(policy, restored, 1_700 + 3_600).expect("固定正文必须可构造");
    let DeadmanDecision::Fire { alert: second, .. } = decision else {
        panic!("冷却结束后必须再报");
    };
    assert_eq!(first.key(), second.key());
}

#[test]
fn 跨重启恢复的幂等键被调度器识别为已送达因而不重复打扰() {
    let fixture = fixture();
    let policy = DeadmanPolicy::new(600, 60, 3_600, 3).expect("固定测试参数必须合法");
    let store = DeadmanStore::open(&fixture.root).expect("私有目录必须可绑定");
    let start = DeadmanState::started_at(1_000).with_receipt(1_000);
    let (fired, decision) = evaluate(policy, start, 1_700).expect("固定正文必须可构造");
    let DeadmanDecision::Fire { alert, .. } = decision else {
        panic!("超期必须触发");
    };
    store.save(&fired).expect("保存必须成功");

    let ledger = MemoryAuditLedger::new();
    let transport = ScriptedTransport::always_ok();
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");
    dispatcher.remember(alert.key().clone());

    let outcome = dispatcher.dispatch(&ledger, &transport, &alert, fired.dispatch_round);

    assert_eq!(
        outcome,
        studiolo_observability::alert::DispatchOutcome::AlreadyDelivered
    );
    assert_eq!(transport.calls(), 0);
}
