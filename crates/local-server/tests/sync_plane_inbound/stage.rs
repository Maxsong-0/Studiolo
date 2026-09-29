//! 入站与删除保全共享内容地址，引用最大值和安装副本都必须跨重开成立。

use super::fixture::{Fixture, NOW, SEQUENCE, Source, path, version, view};
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_core::VaultPath;
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::content::inbound::freeze_source;
use studiolo_local_server::sync_plane::mirror::stage::{
    OBJECT_MAX_BYTES, RETENTION_SECONDS, StageStore,
};
use studiolo_local_server::sync_plane::mirror::{install, remove};
use studiolo_sync_bridge::executor::ObjectId;

#[test]
fn preserved_reference_never_shrinks_sequence_or_retention_after_reopen() {
    let bytes = b"shared source";
    let f = Fixture::new(bytes);
    let version = version(bytes);
    let id = ObjectId::from_digest(version.sha256);
    let mut stage = f.stage();
    f.freeze(&mut stage, &version);
    // 新序号带旧时间、旧序号带新时间，再来更老的引用，两维都不得倒退。
    for (sequence, now) in [(SEQUENCE + 2, NOW - 10), (1, NOW + 20), (0, NOW - 20)] {
        freeze_source(
            &mut stage,
            &mut Source::new(f.root.join("mirror")),
            &path(),
            &version,
            sequence,
            now,
        )
        .unwrap();
    }
    drop(stage);
    let mut stage = f.stage();
    let entry = &stage.index().preserved[0];
    assert_eq!(entry.plan_sequence, SEQUENCE + 2);
    assert_eq!(entry.preserved_at_unix, NOW + 20);
    let retention = i64::try_from(RETENTION_SECONDS).unwrap();
    assert_eq!(
        stage.sweep(NOW + retention * 30, SEQUENCE + 1).unwrap(),
        0,
        "即便远超七天，未退休引用不能清"
    );
    assert_eq!(
        stage.read_preserved(id, version.bytes, 1024).unwrap(),
        bytes
    );
    assert_eq!(
        stage.sweep(NOW + retention + 19, SEQUENCE + 2).unwrap(),
        0,
        "最新引用保留期未到"
    );
    assert_eq!(stage.sweep(NOW + retention + 20, SEQUENCE + 2).unwrap(), 1);
    assert!(!stage.object_path(id).exists());
}

#[test]
fn outbound_install_keeps_inbound_and_deleted_sources_as_independent_objects() {
    for deleted in [false, true] {
        let bytes = b"retained before install";
        let f = Fixture::new(bytes);
        let version = version(bytes);
        let mut stage = f.stage();
        let mirror = view(&f.root.join("mirror"));
        let id = if deleted {
            remove(&mirror, &mut stage, &path(), &version, SEQUENCE, NOW).unwrap()
        } else {
            f.freeze(&mut stage, &version);
            ObjectId::from_digest(version.sha256)
        };
        let target = VaultPath::parse("notes/outbound.txt").unwrap();
        assert_eq!(
            install(&mirror, &mut stage, id, &target, None).unwrap(),
            version
        );
        assert_eq!(
            stage.read_preserved(id, version.bytes, 1024).unwrap(),
            bytes
        );
        // 视图里可变的安装件不能与私有保全件共享 inode。
        fs::write(
            f.root.join("mirror/notes/outbound.txt"),
            b"later remote body",
        )
        .unwrap();
        drop(stage);
        let stage = f.stage();
        assert_eq!(
            stage.read_preserved(id, version.bytes, 1024).unwrap(),
            bytes
        );
        assert!(
            stage
                .index()
                .contains(&studiolo_local_server::sync_plane::mirror::stage::hex(
                    id.digest()
                ))
        );
    }
}

fn sparse(path: &std::path::Path, bytes: u64) {
    let file = fs::File::create(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    file.set_len(bytes).unwrap();
}

#[test]
fn retained_and_orphan_objects_count_toward_existing_stage_budgets() {
    let f = Fixture::new(b"budget");
    let mut stage = f.stage();
    let objects = f.root.join("plane/stage/objects");
    for index in 0..64 {
        sparse(&objects.join(format!("{index:064x}")), 0);
    }
    assert_eq!(
        stage.begin(),
        Err(PlaneError::Storage),
        "64 件包含孤儿，不能再创建接收件"
    );
    drop(stage);
    // 重开清掉上述孤儿，再用稀疏夹具覆盖 512 MiB 总量边界，不实际分配大正文。
    let mut stage = f.stage();
    for index in 0..2 {
        sparse(
            &objects.join(format!("{index:064x}")),
            OBJECT_MAX_BYTES as u64,
        );
    }
    stage.begin().unwrap();
    assert_eq!(stage.accept(b"x"), Err(PlaneError::Storage));
    stage.discard().unwrap();
    sparse(&objects.join("2".repeat(64)), OBJECT_MAX_BYTES as u64 + 1);
    assert_eq!(
        stage.begin(),
        Err(PlaneError::Storage),
        "单件超 256 MiB 也拒绝"
    );
}

#[test]
fn maintenance_open_is_read_only_and_requires_an_existing_index() {
    let f = Fixture::new(b"maintenance");
    let orphan = f.root.join("plane/stage/objects/unreferenced");
    sparse(&orphan, 1);
    let plane = f.root.join("plane");
    let opened = StageStore::open_existing_for_maintenance(&plane).unwrap();
    assert!(orphan.exists(), "维护检查不得回收孤儿");
    drop(opened);
    fs::remove_file(plane.join("stage/index.json")).unwrap();
    assert!(matches!(
        StageStore::open_existing_for_maintenance(&plane),
        Err(PlaneError::Storage)
    ));
    assert!(orphan.exists());
}

#[test]
fn corrupted_duplicate_address_cannot_replace_a_preserved_source() {
    let bytes = b"same bytes";
    let f = Fixture::new(bytes);
    let version = version(bytes);
    let mut stage = f.stage();
    f.freeze(&mut stage, &version);
    let id = ObjectId::from_digest(version.sha256);
    fs::write(stage.object_path(id), b"evil bytes").unwrap();
    assert_eq!(
        freeze_source(
            &mut stage,
            &mut Source::new(f.root.join("mirror")),
            &path(),
            &version,
            SEQUENCE,
            NOW
        ),
        Err(PlaneError::Storage)
    );
    assert_eq!(
        stage.verify_preserved(id, version.bytes),
        Err(PlaneError::Storage)
    );
}
