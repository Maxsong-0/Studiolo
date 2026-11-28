//! 同步状态回执的事实来源与真实落盘（T-224）：「最后对上」只在空闲、一批闭合时前移；
//! 重启沿用上一份回执，首装从打开时刻起算，坏了记空；被挡数只在走到守卫的轮次连同那一轮时刻换新；
//! 回执是本人 0600，写它不动 `plane-state.json`；每轮写回执要有操作许可、拿不到租约就跳过。
use super::*;
use crate::sync_plane::recovery::{PlaneStateStore, SYNC_STATUS_FILE, SYNC_STATUS_STAGE_FILE};
use crate::sync_plane::status::{self, Resumed, StatusReceipt};
use crate::sync_plane::{publish_round_status, publish_status, resume_status};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

const NOW: i64 = 1_800_000_000;

/// 一种把回执弄坏的办法：名字、动手的函数。
type Corruption = (&'static str, fn(&Path));

fn resumed(last_aligned_at: Option<i64>, deletes_gate: Option<(u64, i64)>) -> Resumed {
    Resumed {
        last_aligned_at,
        deletes_gate,
    }
}

/// 回执里成对的那两项。
fn gate(receipt: StatusReceipt) -> (Option<u64>, Option<i64>) {
    (receipt.deletes_blocked, receipt.deletes_checked_at)
}

/// 契约示例同形、内容合法的回执字节（坏文件用例只靠权限或链接让它读不出来）。
const VALID: &[u8] = br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0,"deletes_checked_at":5}"#;

fn plane_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

fn on_disk(root: &Path) -> StatusReceipt {
    let path = root.join(SYNC_STATUS_FILE);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    status::decode(&fs::read(path).unwrap()).expect("写出的回执必须读得回")
}

/// 落盘用例用真实时钟：`publish_status` 的写出时刻取的就是它。
fn clock() -> i64 {
    crate::sync_plane::health::now_unix_seconds()
}

fn private_file(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn 最后对上只在空闲和一批闭合时前移() {
    let health = PlaneHealth::new(None);
    health.resume_status(resumed(Some(1_000), None));
    for (at, outcome) in [
        (2_000, Ok(RoundOutcome::Skipped)),
        (2_300, Ok(RoundOutcome::Recovered)),
        (2_600, unknown(11)),
        (2_900, blocked(11)),
        (3_200, Ok(RoundOutcome::DeletesBlocked { count: 1309 })),
        (3_500, Err(PlaneError::Fused)),
        (3_800, Err(PlaneError::Storage)),
        (4_100, Err(PlaneError::Transport)),
    ] {
        health.record_round(span(at), &outcome);
        assert_eq!(
            health.status_receipt(at + 1).last_aligned_at,
            Some(1_000),
            "{outcome:?}"
        );
    }
    health.record_round(span(5_000), &Ok(RoundOutcome::Idle));
    assert_eq!(health.status_receipt(5_001).last_aligned_at, Some(5_000));
    health.record_round(span(5_300), &settled(12));
    assert_eq!(health.status_receipt(5_301).last_aligned_at, Some(5_300));
    health.record_round(span(5_600), &unknown(13));
    let receipt = health.status_receipt(5_601);
    assert_eq!(receipt.last_aligned_at, Some(5_300));
    assert_eq!(
        (receipt.schema, receipt.written_at),
        (status::SCHEMA, 5_601)
    );
}

#[test]
fn 被挡数只在走到守卫的轮次连同那一轮时刻换新且沿用值不进第六节() {
    let health = PlaneHealth::new(None);
    health.resume_status(resumed(Some(NOW - 7 * 3600), Some((1309, NOW - 60))));
    assert_eq!(
        gate(health.status_receipt(NOW)),
        (Some(1309), Some(NOW - 60))
    );
    // 健康第六节照旧只认本进程守卫证明过的（T-222 的语义不变）。
    assert_eq!(health.bridge().deletes_blocked, None);
    // 走不到守卫的轮次（退避、恢复、终态未知、观察前出错）不换新：证明时刻停在原处，回执刷新也不给它续命（复查 F5）。
    for (at, outcome) in [
        (NOW + 300, Ok(RoundOutcome::Skipped)),
        (NOW + 600, Ok(RoundOutcome::Recovered)),
        (NOW + 900, unknown(11)),
        (NOW + 1_200, Err(PlaneError::Observation)),
    ] {
        health.record_round(span(at), &outcome);
        assert_eq!(
            gate(health.status_receipt(at + 1)),
            (Some(1309), Some(NOW - 60)),
            "{outcome:?}"
        );
    }
    // 守卫在轮内给结论，轮末收口才落成一对，时刻取这一轮的。
    health.record_delete_gate(0);
    assert_eq!(
        gate(health.status_receipt(NOW + 1_500)),
        (Some(1309), Some(NOW - 60))
    );
    health.record_round(span(NOW + 1_500), &Ok(RoundOutcome::Idle));
    assert_eq!(
        gate(health.status_receipt(NOW + 1_501)),
        (Some(0), Some(NOW + 1_500))
    );
    assert_eq!(health.bridge().deletes_blocked, Some(0));
    health.record_delete_gate(1310);
    health.record_round(
        span(NOW + 1_800),
        &Ok(RoundOutcome::DeletesBlocked { count: 1310 }),
    );
    health.record_round(span(NOW + 2_100), &Err(PlaneError::Transport));
    assert_eq!(
        gate(health.status_receipt(NOW + 2_101)),
        (Some(1310), Some(NOW + 1_800))
    );
    // 证明时刻晚于写出时刻（时钟回拨）：两项一起写空，不写出读不回的回执。
    assert_eq!(gate(health.status_receipt(NOW + 1_799)), (None, None));
    // 接上一份记空的回执：说不清就是说不清，不拿打开时刻冒充。
    let fresh = PlaneHealth::new(None);
    fresh.resume_status(resumed(None, None));
    assert_eq!(fresh.status_receipt(NOW).last_aligned_at, None);
    // 时钟回拨：对上时刻晚于写出时刻，写「说不清」，不写出自己读不回的回执。
    let rolled = PlaneHealth::new(None);
    rolled.resume_status(resumed(Some(NOW), None));
    assert_eq!(rolled.status_receipt(NOW - 1).last_aligned_at, None);
    assert_eq!(rolled.status_receipt(NOW).last_aligned_at, Some(NOW));
    // 没接回执的累积器（只在测试里出现）同样写「说不清」。
    assert_eq!(
        PlaneHealth::new(None).status_receipt(NOW).last_aligned_at,
        None
    );
}

#[test]
fn 首装没有回执从打开时刻起算并立刻写出() {
    let root = plane_root();
    let store = PlaneStateStore::open(root.path()).unwrap();
    let health = PlaneHealth::new(None);
    let now = clock();
    resume_status(&store, &health, now);
    publish_status(&store, &health);
    let written = on_disk(root.path());
    assert_eq!(written.last_aligned_at, Some(now));
    assert_eq!(gate(written), (None, None));
    assert!(written.written_at >= now);
    assert!(!root.path().join(SYNC_STATUS_STAGE_FILE).exists());
}

#[test]
fn 重启沿用上一份回执里的对上时刻和被挡数() {
    let root = plane_root();
    let now = clock();
    let before = StatusReceipt {
        schema: status::SCHEMA,
        written_at: now - 60,
        last_aligned_at: Some(now - 7 * 3600),
        deletes_blocked: Some(1309),
        deletes_checked_at: Some(now - 60),
    };
    {
        let store = PlaneStateStore::open(root.path()).unwrap();
        store
            .write_status(&status::encode(&before).unwrap())
            .unwrap();
    }
    let store = PlaneStateStore::open(root.path()).unwrap();
    let health = PlaneHealth::new(None);
    resume_status(&store, &health, now);
    publish_status(&store, &health);
    let after = on_disk(root.path());
    assert_eq!(after.last_aligned_at, Some(now - 7 * 3600));
    assert_eq!(gate(after), (Some(1309), Some(now - 60)));
}

#[test]
fn 回执坏了记空并换成一份合法回执() {
    let cases: [Corruption; 4] = [
        ("内容不合法", |path| private_file(path, b"not-json")),
        ("字段不闭合", |path| {
            private_file(path, br#"{"schema":1,"written_at":10,"last_aligned_at":5}"#);
        }),
        ("别人能读", |path| {
            private_file(path, VALID);
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }),
        ("是链接", |path| {
            let target = path.with_file_name("elsewhere.json");
            private_file(&target, VALID);
            std::os::unix::fs::symlink(target, path).unwrap();
        }),
    ];
    for (name, corrupt) in cases {
        let root = plane_root();
        corrupt(&root.path().join(SYNC_STATUS_FILE));
        let store = PlaneStateStore::open(root.path()).unwrap();
        let health = PlaneHealth::new(None);
        resume_status(&store, &health, clock());
        assert_eq!(
            health.status_receipt(clock()).last_aligned_at,
            None,
            "{name}"
        );
        publish_status(&store, &health);
        let replaced = on_disk(root.path());
        assert_eq!(replaced.last_aligned_at, None, "{name}");
        assert!(
            !fs::symlink_metadata(root.path().join(SYNC_STATUS_FILE))
                .unwrap()
                .file_type()
                .is_symlink(),
            "{name}"
        );
    }
}

#[test]
fn 写回执不碰持久状态且清掉陈旧暂存件() {
    let root = plane_root();
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    let state = store.state().clone();
    store.save(state).unwrap();
    let plane_state = root.path().join("plane-state.json");
    let bytes = fs::read(&plane_state).unwrap();
    private_file(&root.path().join(SYNC_STATUS_STAGE_FILE), b"half");
    let health = PlaneHealth::new(None);
    let now = clock();
    resume_status(&store, &health, now - 600);
    health.record_round(span(now), &Ok(RoundOutcome::Idle));
    publish_status(&store, &health);
    assert_eq!(fs::read(&plane_state).unwrap(), bytes);
    assert_eq!(on_disk(root.path()).last_aligned_at, Some(now));
    assert!(!root.path().join(SYNC_STATUS_STAGE_FILE).exists());
    assert!(
        store
            .write_status(&vec![b' '; status::MAX_BYTES + 1])
            .is_err()
    );
    assert_eq!(on_disk(root.path()).last_aligned_at, Some(now));
    // 上限两侧：正好上限写得进（这份字节不是回执，只证明写盘不多拒一个字节）。
    store.write_status(&vec![b' '; status::MAX_BYTES]).unwrap();
    assert_eq!(
        fs::metadata(root.path().join(SYNC_STATUS_FILE))
            .unwrap()
            .len(),
        status::MAX_BYTES as u64
    );
}

#[test]
fn 合法回执照原样接上作为坏文件用例的对照() {
    let root = plane_root();
    private_file(&root.path().join(SYNC_STATUS_FILE), VALID);
    let store = PlaneStateStore::open(root.path()).unwrap();
    let health = PlaneHealth::new(None);
    resume_status(&store, &health, clock());
    let receipt = health.status_receipt(clock());
    assert_eq!(receipt.last_aligned_at, Some(5));
    assert_eq!(gate(receipt), (Some(0), Some(5)));
}

#[test]
fn 每轮写回执要有操作许可拿不到租约就跳过() {
    let root = plane_root();
    let health = PlaneHealth::new(None);
    health.resume_status(resumed(Some(clock()), None));
    // 冻结正占着全局操作许可：不碰平面租约，也不写。
    publish_round_status(None::<()>, root.path(), &health);
    assert!(!root.path().join(SYNC_STATUS_FILE).exists());
    // 有许可、但租约被别人持着（同一把 flock 按打开文件描述计）：跳过，不 panic、不写。
    let holder = PlaneStateStore::open(root.path()).unwrap();
    publish_round_status(Some(()), root.path(), &health);
    assert!(!root.path().join(SYNC_STATUS_FILE).exists());
    // 租约放开后照常写出一份合法回执。
    drop(holder);
    publish_round_status(Some(()), root.path(), &health);
    assert!(on_disk(root.path()).last_aligned_at.is_some());
}
