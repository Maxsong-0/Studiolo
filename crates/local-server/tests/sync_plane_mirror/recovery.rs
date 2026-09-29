//! 恢复协议与排除项守卫的契约：首次打开默认干净、已启动未回收强制恢复、rc=1 退避阶梯、
//! 撕裂失败关闭、租约排他、残留守卫三态；版本 1 追加字段的兼容与失败关闭。

use crate::fixture::private_dir;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::recovery::{
    PLANE_LOCK, PLANE_STATE_FILE, PlaneState, PlaneStateStore, backoff_seconds, decide,
    mark_reaped, mark_started, residue_guard,
};
use studiolo_sync_transport::owned_run::SyncMode;

#[test]
fn state_first_open_is_clean() {
    let root = private_dir();
    let store = PlaneStateStore::open(root.path()).unwrap();
    let state = store.state();
    assert!(state.last_run_clean);
    assert_eq!(state.started_sequence, None);
    assert!(state.verified_excluded.is_empty());
    assert_eq!(state.applied_mode, None);
    assert_eq!(state, &PlaneState::first_run());
    let decision = decide(state, SyncMode::Bidirectional, 0);
    assert_eq!(
        (
            decision.effective_mode,
            decision.forced_recovery,
            decision.backoff
        ),
        (SyncMode::Bidirectional, false, false)
    );
    assert!(root.path().join(PLANE_LOCK).exists());
    assert!(
        !root.path().join(PLANE_STATE_FILE).exists(),
        "首次打开不写状态文件"
    );
}

#[test]
fn state_started_without_reap_forces_recovery_after_reopen() {
    let root = private_dir();
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    let mut started = mark_started(store.state(), 7);
    started.verified_excluded = ["School", "Archive", "School"].map(str::to_owned).to_vec();
    started.applied_mode = Some(SyncMode::Bidirectional);
    store.save(started).unwrap();
    drop(store);
    let mut reopened = PlaneStateStore::open(root.path()).unwrap();
    assert_eq!(reopened.state().started_sequence, Some(7));
    assert_eq!(
        reopened.state().verified_excluded,
        vec!["Archive", "School"],
        "排序去重后持久化"
    );
    assert_eq!(reopened.state().applied_mode, Some(SyncMode::Bidirectional));
    let decision = decide(reopened.state(), SyncMode::Bidirectional, 0);
    assert!(decision.forced_recovery);
    assert_eq!(decision.effective_mode, SyncMode::MirrorRemote);
    let reaped = mark_reaped(reopened.state(), true, false, 100);
    reopened.save(reaped).unwrap();
    drop(reopened);
    let clean = PlaneStateStore::open(root.path()).unwrap();
    assert!(!decide(clean.state(), SyncMode::Bidirectional, 100).forced_recovery);
    assert_eq!(clean.state().last_exit_unix, Some(100));
    assert_eq!(clean.state().started_sequence, None);
}

#[test]
fn state_rc_one_backs_off_along_ladder_and_caps() {
    let now = 1_000;
    let mut state = PlaneState::first_run();
    let ladder = [5, 10, 20, 40, 80, 160, 300, 300];
    for (index, gap) in ladder.iter().enumerate() {
        state = mark_reaped(&state, false, true, now);
        assert_eq!(state.consecutive_rc1, u32::try_from(index).unwrap() + 1);
        assert_eq!(state.not_before_unix, Some(now + gap));
        assert!(!state.last_run_clean);
        assert!(decide(&state, SyncMode::Bidirectional, now + gap - 1).backoff);
        assert!(!decide(&state, SyncMode::Bidirectional, now + gap).backoff);
        assert!(decide(&state, SyncMode::Bidirectional, now).forced_recovery);
    }
    assert_eq!(backoff_seconds(0), 5);
    assert_eq!(backoff_seconds(7), 300);
    assert_eq!(backoff_seconds(u32::MAX), 300);
    let other = mark_reaped(&state, false, false, now);
    assert_eq!(
        (other.consecutive_rc1, other.not_before_unix),
        (0, Some(now + 5)),
        "其它非干净退出：只等 5 s，连续计数归零"
    );
    let clean = mark_reaped(&state, true, false, now);
    assert_eq!(
        (
            clean.consecutive_rc1,
            clean.not_before_unix,
            clean.last_run_clean
        ),
        (0, None, true)
    );
}

#[test]
fn state_torn_file_fails_closed() {
    let root = private_dir();
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    store.save(mark_started(store.state(), 1)).unwrap();
    drop(store);
    let file = root.path().join(PLANE_STATE_FILE);
    let intact = fs::read(&file).unwrap();
    let mut torn = intact.clone();
    torn.pop();
    fs::write(&file, &torn).unwrap();
    assert_eq!(
        PlaneStateStore::open(root.path()).err(),
        Some(PlaneError::Storage),
        "撕裂"
    );
    let edited = String::from_utf8(intact.clone())
        .unwrap()
        .replace("\"consecutive_rc1\":0", "\"consecutive_rc1\":3");
    fs::write(&file, edited).unwrap();
    assert_eq!(
        PlaneStateStore::open(root.path()).err(),
        Some(PlaneError::Storage),
        "校验和不符"
    );
    fs::write(&file, &intact).unwrap();
    assert_eq!(
        PlaneStateStore::open(root.path())
            .unwrap()
            .state()
            .started_sequence,
        Some(1)
    );
}

#[test]
fn state_lock_is_exclusive() {
    let root = private_dir();
    let held = PlaneStateStore::open(root.path()).unwrap();
    assert_eq!(
        PlaneStateStore::open(root.path()).err(),
        Some(PlaneError::Storage)
    );
    drop(held);
    assert!(PlaneStateStore::open(root.path()).is_ok());
}

#[test]
fn residue_guard_reports_three_states() {
    let mirror = private_dir();
    let previous = ["School".to_owned(), "Archive/Old".to_owned()];
    let current = ["Archive/Old".to_owned()];
    assert_eq!(
        residue_guard(&previous, &current, mirror.path()).unwrap(),
        None,
        "文件夹不存在"
    );
    fs::create_dir(mirror.path().join("School")).unwrap();
    assert_eq!(
        residue_guard(&previous, &current, mirror.path()).unwrap(),
        None,
        "空目录不算残留"
    );
    fs::write(mirror.path().join("School/a.md"), b"local copy").unwrap();
    assert_eq!(
        residue_guard(&previous, &current, mirror.path()).unwrap(),
        Some("School".to_owned()),
        "非空即残留"
    );
    fs::create_dir_all(mirror.path().join("Archive/Old")).unwrap();
    fs::write(mirror.path().join("Archive/Old/b.md"), b"still excluded").unwrap();
    assert_eq!(
        residue_guard(&["Archive/Old".to_owned()], &current, mirror.path()).unwrap(),
        None,
        "仍在当前排除集合里的文件夹不查"
    );
    assert_eq!(
        residue_guard(&["Archive/Old".to_owned()], &[], mirror.path()).unwrap(),
        Some("Archive/Old".to_owned()),
        "多级文件夹逐段下探"
    );
    fs::write(mirror.path().join("Plain"), b"a file where a folder was").unwrap();
    assert_eq!(
        residue_guard(&["Plain".to_owned()], &[], mirror.path()).unwrap(),
        Some("Plain".to_owned()),
        "名字被文件占着也算残留"
    );
    assert_eq!(
        residue_guard(&previous, &current, &mirror.path().join("missing")).err(),
        Some(PlaneError::Unavailable)
    );
}

/// 追加字段之前的校验和算法（独立重写作对照）：追加字段为空时，新实现必须算出同一个值。
fn legacy_checksum(state: &PlaneState) -> String {
    let mut digest = Sha256::new();
    digest.update(b"studiolo.sync-plane.state.v1\0");
    digest.update([1, u8::from(state.last_run_clean)]);
    digest.update([u8::from(state.started_sequence.is_some())]);
    digest.update(state.started_sequence.unwrap_or_default().to_be_bytes());
    digest.update([u8::from(state.last_exit_unix.is_some())]);
    digest.update(state.last_exit_unix.unwrap_or_default().to_be_bytes());
    digest.update(state.consecutive_rc1.to_be_bytes());
    digest.update([u8::from(state.not_before_unix.is_some())]);
    digest.update(state.not_before_unix.unwrap_or_default().to_be_bytes());
    digest.update((state.verified_excluded.len() as u64).to_be_bytes());
    for folder in &state.verified_excluded {
        digest.update((folder.len() as u64).to_be_bytes());
        digest.update(folder.as_bytes());
    }
    digest.update([match state.applied_mode {
        None => 0,
        Some(SyncMode::MirrorRemote) => 1,
        Some(SyncMode::Bidirectional) => 2,
    }]);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn state_new_fields_round_trip_and_the_old_format_still_decodes() {
    let root = private_dir();
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    // 旧格式：追加字段为空时不写出，校验和与旧算法逐字节相同，照常解码。
    let mut old = mark_reaped(&mark_started(store.state(), 3), false, true, 1_000);
    old.verified_excluded = vec!["Archive".to_owned()];
    old.applied_mode = Some(SyncMode::MirrorRemote);
    store.save(old).unwrap();
    let old = store.state().clone();
    assert_eq!(old.checksum, legacy_checksum(&old));
    drop(store);
    let text = fs::read_to_string(root.path().join(PLANE_STATE_FILE)).unwrap();
    assert!(!text.contains("headless_excluded"));
    assert!(!text.contains("conflict_stamps"));
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    assert_eq!(store.state(), &old);
    // 两个新字段落盘再读回一致；headless 集合排序去重，非空即进校验和。
    let mut next = old.clone();
    next.headless_excluded = ["School", "Archive", "School"].map(str::to_owned).to_vec();
    next.conflict_stamps = BTreeMap::from([("a".repeat(64), "202609261200".to_owned())]);
    store.save(next).unwrap();
    let saved = store.state().clone();
    drop(store);
    let reopened = PlaneStateStore::open(root.path()).unwrap();
    assert_eq!(reopened.state(), &saved);
    assert_eq!(saved.headless_excluded, vec!["Archive", "School"]);
    assert_eq!(saved.conflict_stamps.len(), 1);
    assert_ne!(saved.checksum, legacy_checksum(&saved));
}

#[test]
fn state_malformed_conflict_stamps_fail_closed_on_save_and_on_open() {
    let root = private_dir();
    let file = root.path().join(PLANE_STATE_FILE);
    let (key, stamp) = ("b".repeat(64), "202609261200".to_owned());
    let malformed = [
        ("B".repeat(64), stamp.clone()),
        ("b".repeat(63), stamp.clone()),
        (key.clone(), "20260926120".to_owned()),
        (key.clone(), "2026092612ab".to_owned()),
    ];
    for (bad_key, bad_stamp) in malformed {
        let mut store = PlaneStateStore::open(root.path()).unwrap();
        let mut state = store.state().clone();
        state.conflict_stamps = BTreeMap::from([(bad_key.clone(), bad_stamp.clone())]);
        // 写盘前自检：畸形状态不落盘。
        assert_eq!(
            store.save(state.clone()).err(),
            Some(PlaneError::Storage),
            "{bad_key} {bad_stamp}"
        );
        drop(store);
        // 绕过自检写出一份校验和自洽的畸形文件（`mark_started` 会补齐校验和）：打开即失败关闭。
        let sealed = serde_json::to_vec(&mark_started(&state, 1)).unwrap();
        fs::write(&file, sealed).unwrap();
        assert_eq!(
            PlaneStateStore::open(root.path()).err(),
            Some(PlaneError::Storage),
            "{bad_key} {bad_stamp}"
        );
        fs::remove_file(&file).unwrap();
    }
    // 条目超过 512 同样拒绝落盘。
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    let mut state = store.state().clone();
    state.conflict_stamps = (0..513)
        .map(|index| (format!("{index:064x}"), stamp.clone()))
        .collect();
    assert_eq!(store.save(state).err(), Some(PlaneError::Storage));
}
