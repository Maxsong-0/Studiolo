//! 同步状态回执的事实来源与真实落盘（T-224）：「最后对上」只在空闲、一批闭合时前移；
//! 重启沿用上一份回执，首装从打开时刻起算，坏了记空；回执是本人 0600，写它不动 `plane-state.json`。
use super::*;
use crate::sync_plane::recovery::{PlaneStateStore, SYNC_STATUS_FILE, SYNC_STATUS_STAGE_FILE};
use crate::sync_plane::status::{self, Resumed, StatusReceipt};
use crate::sync_plane::{publish_status, resume_status};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

const NOW: i64 = 1_800_000_000;

/// 一种把回执弄坏的办法：名字、动手的函数。
type Corruption = (&'static str, fn(&Path));

fn resumed(last_aligned_at: Option<i64>, deletes_blocked: Option<u64>) -> Resumed {
    Resumed {
        last_aligned_at,
        deletes_blocked,
    }
}

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
        health.record_round(at, &outcome);
        assert_eq!(
            health.status_receipt(at + 1).last_aligned_at,
            Some(1_000),
            "{outcome:?}"
        );
    }
    health.record_round(5_000, &Ok(RoundOutcome::Idle));
    assert_eq!(health.status_receipt(5_001).last_aligned_at, Some(5_000));
    health.record_round(5_300, &settled(12));
    assert_eq!(health.status_receipt(5_301).last_aligned_at, Some(5_300));
    health.record_round(5_600, &unknown(13));
    let receipt = health.status_receipt(5_601);
    assert_eq!(receipt.last_aligned_at, Some(5_300));
    assert_eq!(
        (receipt.schema, receipt.written_at),
        (status::SCHEMA, 5_601)
    );
}

#[test]
fn 被挡数守卫结论优先没跑过才沿用上一个进程的且不进第六节() {
    let health = PlaneHealth::new(None);
    health.resume_status(resumed(Some(NOW), Some(1309)));
    assert_eq!(health.status_receipt(NOW).deletes_blocked, Some(1309));
    // 健康第六节照旧只认本进程守卫证明过的（T-222 的语义不变）。
    assert_eq!(health.bridge().deletes_blocked, None);
    health.record_delete_gate(0);
    assert_eq!(health.status_receipt(NOW).deletes_blocked, Some(0));
    health.record_delete_gate(1310);
    assert_eq!(health.status_receipt(NOW).deletes_blocked, Some(1310));
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
    assert_eq!(written.deletes_blocked, None);
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
    assert_eq!(after.deletes_blocked, Some(1309));
}

#[test]
fn 回执坏了记空并换成一份合法回执() {
    let cases: [Corruption; 4] = [
        ("内容不合法", |path| private_file(path, b"not-json")),
        ("字段不闭合", |path| {
            private_file(path, br#"{"schema":1,"written_at":10,"last_aligned_at":5}"#);
        }),
        ("别人能读", |path| {
            private_file(
                path,
                br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0}"#,
            );
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }),
        ("是链接", |path| {
            let target = path.with_file_name("elsewhere.json");
            private_file(
                &target,
                br#"{"schema":1,"written_at":10,"last_aligned_at":5,"deletes_blocked":0}"#,
            );
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
    health.record_round(now, &Ok(RoundOutcome::Idle));
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
}
