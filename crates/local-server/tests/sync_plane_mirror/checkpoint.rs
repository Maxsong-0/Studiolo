//! 真实文件与原平面租约的封存契约；不冒充受控传输或完整七 owner 验收。
use crate::fixture::sha256;
use std::{
    fs,
    os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink},
    time::{Duration, Instant},
};
use studiolo_local_server::sync_plane::{
    PlaneError,
    mirror::stage::{StageStore, hex},
    recovery::{
        FrozenPlaneFiles, PLANE_STATE_FILE, PendingPurge, PlaneStateStore, mark_reaped,
        mark_started,
    },
};
use studiolo_sync_bridge::executor::ObjectId;

// testbox 的 /tmp 是小容量 tmpfs；真正导出使用作业 TMPDIR，保留生产 2 GiB 余量门槛。
fn private_dir() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}
fn until() -> Instant {
    Instant::now() + Duration::from_secs(20)
}
fn fixture() -> (tempfile::TempDir, ObjectId) {
    let root = private_dir();
    StageStore::initialize(root.path()).unwrap();
    let mut stage = StageStore::open(root.path()).unwrap();
    let id = ObjectId::from_digest(sha256(b"retained original body"));
    stage.begin().unwrap();
    stage.accept(b"retained original body").unwrap();
    stage.finish(id).unwrap();
    stage.preserve(id, 7, "笔记/原文.md", 100).unwrap();
    let mut state = PlaneStateStore::open(root.path()).unwrap();
    state
        .save(mark_reaped(state.state(), false, true, 200))
        .unwrap();
    (root, id)
}
fn private_file(path: &std::path::Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn checkpoint_preserves_raw_state_all_objects_and_original_lease() {
    let (root, id) = fixture();
    let target = private_dir();
    let frozen = FrozenPlaneFiles::open(root.path(), until()).unwrap();
    assert!(PlaneStateStore::open(root.path()).is_err());
    frozen.export(target.path()).unwrap();
    assert!(PlaneStateStore::open(root.path()).is_err());
    assert_eq!(
        fs::read(root.path().join(PLANE_STATE_FILE)).unwrap(),
        fs::read(target.path().join(PLANE_STATE_FILE)).unwrap()
    );
    let copied = StageStore::open_existing_for_maintenance(target.path()).unwrap();
    assert_eq!(
        copied.read_preserved(id, 22, 1024).unwrap(),
        b"retained original body"
    );
    let object = format!("stage/objects/{}", hex(id.digest()));
    for name in [PLANE_STATE_FILE, "stage/index.json", &object] {
        let source = fs::metadata(root.path().join(name)).unwrap();
        let copy = fs::metadata(target.path().join(name)).unwrap();
        assert_ne!((source.dev(), source.ino()), (copy.dev(), copy.ino()));
        assert_eq!(copy.mode() & 0o777, 0o600);
        assert_eq!(copy.nlink(), 1);
    }
    assert!(frozen.export(target.path()).is_err());
    assert!(frozen.export(root.path()).is_err());
    drop(frozen);
    assert!(PlaneStateStore::open(root.path()).is_ok());
}
#[test]
fn checkpoint_first_run_preserves_real_absence_without_creating_source_state() {
    let root = private_dir();
    let target = private_dir();
    StageStore::initialize(root.path()).unwrap();
    let frozen = FrozenPlaneFiles::open(root.path(), until()).unwrap();
    frozen.export(target.path()).unwrap();
    assert!(!root.path().join(PLANE_STATE_FILE).exists());
    assert!(!target.path().join(PLANE_STATE_FILE).exists());
    assert!(
        PlaneStateStore::open(target.path())
            .unwrap()
            .state()
            .last_run_clean
    );
}
#[test]
fn checkpoint_rejects_active_or_pending_purge_and_releases_failed_lease() {
    for purge in [false, true] {
        let (root, _) = fixture();
        let mut store = PlaneStateStore::open(root.path()).unwrap();
        let mut state = if purge {
            store.state().clone()
        } else {
            mark_started(store.state(), 9)
        };
        if purge {
            state.pending_purge = Some(PendingPurge {
                event_id: "a".repeat(64),
                folder: "Archive".into(),
                planned_files: 1,
                planned_directories: 1,
                deletion_started: false,
                result: None,
            });
        }
        store.save(state).unwrap();
        drop(store);
        assert!(FrozenPlaneFiles::open(root.path(), until()).is_err());
        assert!(PlaneStateStore::open(root.path()).is_ok());
    }
}
#[test]
fn checkpoint_rejects_orphans_partial_files_and_corrupt_objects_without_cleanup() {
    for name in [
        "stage/objects/orphan",
        "stage/objects/receiving.partial",
        "stage/index.json.new",
        "plane-state.json.new",
    ] {
        let (root, _) = fixture();
        let path = root.path().join(name);
        private_file(&path, b"unclosed");
        assert!(FrozenPlaneFiles::open(root.path(), until()).is_err());
        assert!(path.exists());
    }
    let (root, id) = fixture();
    let path = root
        .path()
        .join(format!("stage/objects/{}", hex(id.digest())));
    private_file(&path, b"evil same size content");
    assert!(FrozenPlaneFiles::open(root.path(), until()).is_err());
}
#[test]
fn checkpoint_rejects_source_replacement_lock_replacement_and_expiry() {
    for replace_lock in [false, true] {
        let (root, _) = fixture();
        let target = private_dir();
        let frozen = FrozenPlaneFiles::open(root.path(), until()).unwrap();
        let name = if replace_lock {
            "plane.lock"
        } else {
            PLANE_STATE_FILE
        };
        let path = root.path().join(name);
        let original = fs::read(&path).unwrap();
        fs::rename(&path, root.path().join("replaced")).unwrap();
        private_file(&path, &original);
        assert_eq!(frozen.export(target.path()), Err(PlaneError::Storage));
        assert!(fs::read_dir(target.path()).unwrap().next().is_none());
    }
    let (root, _) = fixture();
    assert!(FrozenPlaneFiles::open(root.path(), Instant::now()).is_err());
    let target = private_dir();
    let frozen =
        FrozenPlaneFiles::open(root.path(), Instant::now() + Duration::from_millis(150)).unwrap();
    std::thread::sleep(Duration::from_millis(170));
    assert!(frozen.export(target.path()).is_err());
}
#[test]
fn checkpoint_rejects_symlink_hardlink_nested_and_nonprivate_targets() {
    let (root, _) = fixture();
    let frozen = FrozenPlaneFiles::open(root.path(), until()).unwrap();
    let outside = private_dir();
    let target = private_dir();
    symlink(target.path(), outside.path().join("alias")).unwrap();
    assert!(frozen.export(&outside.path().join("alias")).is_err());
    let nested = root.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(frozen.export(&nested).is_err());
    fs::set_permissions(target.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(frozen.export(target.path()).is_err());
    drop(frozen);
    let path = root.path().join(PLANE_STATE_FILE);
    fs::hard_link(path, outside.path().join("hardlink")).unwrap();
    assert!(FrozenPlaneFiles::open(root.path(), until()).is_err());
}
