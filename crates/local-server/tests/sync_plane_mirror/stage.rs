//! 外 stage 与镜像安装／删除的契约：落库去重、声明字节不符不入账、索引撕裂失败关闭、
//! 孤儿回收、保留期与退休、目标版本核对、删除前保全。

use crate::fixture::{path, private_dir, sha256, version, view, write};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::mirror::stage::{
    INDEX_FILE, OBJECTS_DIR, RECEIVING_FILE, RETENTION_SECONDS, STAGE_DIR, StageStore, hex,
};
use studiolo_local_server::sync_plane::mirror::{StageSink, install, remove};
use studiolo_sync_bridge::executor::{CasFault, CasLimits, MirrorCas, ObjectId};

fn opened() -> (tempfile::TempDir, StageStore) {
    let root = private_dir();
    StageStore::initialize(root.path()).unwrap();
    let store = StageStore::open(root.path()).unwrap();
    (root, store)
}

fn put(store: &mut StageStore, bytes: &[u8]) -> ObjectId {
    let id = ObjectId::from_digest(sha256(bytes));
    store.begin().unwrap();
    store.accept(bytes).unwrap();
    store.finish(id).unwrap();
    id
}

fn objects_dir(root: &Path) -> PathBuf {
    root.join(STAGE_DIR).join(OBJECTS_DIR)
}

fn object_names(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(objects_dir(root))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn private_file(target: &Path, bytes: &[u8]) {
    fs::write(target, bytes).unwrap();
    fs::set_permissions(target, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn stage_initialize_refuses_existing_layout() {
    let (root, store) = opened();
    drop(store);
    assert_eq!(
        StageStore::initialize(root.path()).err(),
        Some(PlaneError::Storage)
    );
    let world_readable = private_dir();
    fs::set_permissions(world_readable.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        StageStore::initialize(world_readable.path()).err(),
        Some(PlaneError::Storage)
    );
}

#[test]
fn stage_dedupes_repeated_objects() {
    let (root, mut store) = opened();
    let id = put(&mut store, b"same bytes");
    let again = put(&mut store, b"same bytes");
    assert_eq!(id, again);
    assert_eq!(object_names(root.path()), vec![hex(id.digest())]);
    assert_eq!(fs::read(store.object_path(id)).unwrap(), b"same bytes");
    assert!(store.index().preserved.is_empty(), "落库本身不登记保全");
    let mismatched = ObjectId::from_digest(sha256(b"other"));
    store.begin().unwrap();
    store.accept(b"same bytes").unwrap();
    assert_eq!(store.finish(mismatched).err(), Some(PlaneError::Storage));
    assert_eq!(
        object_names(root.path()),
        vec![hex(id.digest())],
        "地址不符不入账"
    );
}

#[test]
fn stage_rejects_declared_bytes_mismatch_via_cas() {
    let (root, mut store) = opened();
    let mut cas = MirrorCas::new(CasLimits::internal());
    let expected = ObjectId::from_digest(sha256(b"abc"));
    {
        let mut sink = StageSink::new(&mut store);
        cas.begin_receive(5, Some(expected), &mut sink).unwrap();
        cas.accept_frame(b"abc", &mut sink).unwrap();
        assert_eq!(
            cas.finish_receive(&mut sink),
            Err(CasFault::DeclaredBytesMismatch)
        );
    }
    assert!(!cas.contains(expected));
    assert!(!cas.is_receiving());
    assert!(object_names(root.path()).is_empty(), "不入账也不留临时件");
    store.begin().unwrap();
    store.discard().unwrap();
    assert_eq!(store.discard().err(), Some(PlaneError::Storage));
}

#[test]
fn stage_torn_index_fails_closed() {
    let (root, mut store) = opened();
    let id = put(&mut store, b"keep me");
    store.preserve(id, 3, "笔记/删.md", 1_000_000).unwrap();
    drop(store);
    let index = root.path().join(STAGE_DIR).join(INDEX_FILE);
    let mut bytes = fs::read(&index).unwrap();
    bytes.pop();
    fs::write(&index, bytes).unwrap();
    assert_eq!(
        StageStore::open(root.path()).err(),
        Some(PlaneError::Storage)
    );
}

#[test]
fn stage_open_reclaims_orphans() {
    let (root, store) = opened();
    drop(store);
    let orphan = objects_dir(root.path()).join("0".repeat(64));
    private_file(&orphan, b"nobody references me");
    let stale = objects_dir(root.path()).join(RECEIVING_FILE);
    private_file(&stale, b"half");
    let mut store = StageStore::open(root.path()).unwrap();
    assert!(!orphan.exists(), "索引外的对象在打开时回收");
    assert!(stale.exists(), "接收临时件不算孤儿，留给 begin 处理");
    store.begin().unwrap();
    store.accept(b"x").unwrap();
    store.finish(ObjectId::from_digest(sha256(b"x"))).unwrap();
    assert!(!stale.exists());
    assert_eq!(object_names(root.path()), vec![hex(sha256(b"x"))]);
}

#[test]
fn stage_keeps_preserved_objects_before_retention() {
    let (_root, mut store) = opened();
    let id = put(&mut store, b"retained");
    let now = 1_700_000_000;
    store.preserve(id, 4, "笔记/旧.md", now).unwrap();
    let retention = i64::try_from(RETENTION_SECONDS).unwrap();
    assert_eq!(
        store.sweep(now + retention - 1, 4).unwrap(),
        0,
        "保留期未到"
    );
    assert_eq!(store.sweep(now + retention, 3).unwrap(), 0, "计划未退休");
    assert!(store.object_path(id).exists());
    assert_eq!(store.index().preserved.len(), 1);
}

#[test]
fn stage_sweeps_retired_objects_after_retention() {
    let (root, mut store) = opened();
    let id = put(&mut store, b"expired");
    let now = 1_700_000_000;
    store.preserve(id, 4, "笔记/旧.md", now).unwrap();
    let retention = i64::try_from(RETENTION_SECONDS).unwrap();
    assert_eq!(store.sweep(now + retention, 4).unwrap(), 1);
    assert!(!store.object_path(id).exists());
    assert!(store.index().preserved.is_empty());
    drop(store);
    let reopened = StageStore::open(root.path()).unwrap();
    assert!(
        reopened.index().preserved.is_empty(),
        "索引已原子替换并持久化"
    );
    assert!(object_names(root.path()).is_empty());
}

#[test]
fn install_rejects_target_version_mismatch() {
    let (_plane, mut store) = opened();
    let mirror = private_dir();
    write(mirror.path(), "笔记/旧.md", b"old");
    let id = put(&mut store, b"new");
    let target = path("笔记/旧.md");
    let wrong = version(b"other");
    assert_eq!(
        install(&view(mirror.path()), &mut store, id, &target, Some(&wrong)).err(),
        Some(PlaneError::Observation)
    );
    assert_eq!(
        install(&view(mirror.path()), &mut store, id, &target, None).err(),
        Some(PlaneError::Observation),
        "目标存在而冻结版本说它应缺失"
    );
    assert_eq!(fs::read(mirror.path().join("笔记/旧.md")).unwrap(), b"old");
    assert!(store.object_path(id).exists(), "拒绝时对象仍在 stage");
    let fresh = private_dir();
    assert!(install(&view(fresh.path()), &mut store, id, &target, None).is_ok());
    assert_eq!(
        install(
            &view(fresh.path()),
            &mut store,
            id,
            &target,
            Some(&version(b"new"))
        )
        .err(),
        Some(PlaneError::Storage),
        "对象已被搬走：stage 缺对象"
    );
}

#[test]
fn install_places_bytes_with_matching_digest() {
    let (_plane, mut store) = opened();
    let mirror = private_dir();
    let id = put(&mut store, b"fresh body");
    let target = path("笔记/深 层/新 文件.md");
    let installed = install(&view(mirror.path()), &mut store, id, &target, None).unwrap();
    assert_eq!(installed, version(b"fresh body"));
    let on_disk = mirror.path().join("笔记/深 层/新 文件.md");
    assert_eq!(fs::read(&on_disk).unwrap(), b"fresh body");
    assert!(!store.object_path(id).exists(), "对象已 renameat 进视图");
    let next = put(&mut store, b"second body");
    let replaced = install(
        &view(mirror.path()),
        &mut store,
        next,
        &target,
        Some(&installed),
    )
    .unwrap();
    assert_eq!(replaced, version(b"second body"));
    assert_eq!(fs::read(&on_disk).unwrap(), b"second body");
}

#[test]
fn remove_preserves_bytes_before_unlink() {
    let (root, mut store) = opened();
    let mirror = private_dir();
    write(mirror.path(), "笔记/去.md", b"going away");
    let target = path("笔记/去.md");
    let id = remove(
        &view(mirror.path()),
        &mut store,
        &target,
        &version(b"going away"),
        9,
        1_700_000_000,
    )
    .unwrap();
    assert_eq!(id, ObjectId::from_digest(sha256(b"going away")));
    assert!(!mirror.path().join("笔记/去.md").exists());
    assert_eq!(fs::read(store.object_path(id)).unwrap(), b"going away");
    let entry = &store.index().preserved[0];
    assert_eq!(
        (
            entry.object.as_str(),
            entry.plan_sequence,
            entry.original_path.as_str(),
            entry.bytes,
            entry.preserved_at_unix,
        ),
        (
            hex(id.digest()).as_str(),
            9,
            "笔记/去.md",
            10,
            1_700_000_000
        )
    );
    write(mirror.path(), "笔记/留.md", b"stay");
    assert_eq!(
        remove(
            &view(mirror.path()),
            &mut store,
            &path("笔记/留.md"),
            &version(b"stab"),
            9,
            1
        )
        .err(),
        Some(PlaneError::Observation),
        "同长不同内容：保全中途发现不符，撤销临时件"
    );
    assert!(mirror.path().join("笔记/留.md").exists());
    assert!(!objects_dir(root.path()).join(RECEIVING_FILE).exists());
    assert_eq!(store.index().preserved.len(), 1);
}
