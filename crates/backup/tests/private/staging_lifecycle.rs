//! 只验证原目录生命周期；没有用假 owner 水位冒充完整 checkpoint。
use super::*;
use crate::checkpoint::staging::{StagingArea, creation};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use studiolo_core::checkpoint::CheckpointId;

struct Fixture {
    temp: tempfile::TempDir,
    parent: PathBuf,
    source: PathBuf,
    ephemeral: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        let source = temp.path().join("source");
        let ephemeral = temp.path().join("ephemeral");
        for path in [&parent, &source, &ephemeral] {
            private(path);
        }
        Self {
            temp,
            parent,
            source,
            ephemeral,
        }
    }
    fn area(&self, shared: bool) -> StagingArea {
        let deadline = Instant::now() + Duration::from_secs(30);
        let id = CheckpointId::parse("cleanup").unwrap();
        if shared {
            fs::set_permissions(&self.parent, fs::Permissions::from_mode(0o750)).unwrap();
            StagingArea::create_for_reader(
                &self.parent,
                id,
                std::slice::from_ref(&self.source),
                &self.ephemeral,
                fs::metadata(&self.parent).unwrap().gid(),
                deadline,
            )
            .unwrap()
        } else {
            StagingArea::create(
                &self.parent,
                id,
                std::slice::from_ref(&self.source),
                &self.ephemeral,
                deadline,
            )
            .unwrap()
        }
    }
}
fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
#[test]
fn partial_layout_failure_removes_owned_exports_and_preserves_foreign_collision() {
    let f = Fixture::new();
    let parent = Directory::open(&f.parent, true).unwrap();
    private(&f.parent.join("foreign"));
    fs::write(f.parent.join("foreign/keep"), b"not ours").unwrap();
    assert_eq!(
        creation::layout(
            &parent,
            &CheckpointId::parse("key").unwrap(),
            "export-key",
            "foreign",
            None,
            Instant::now() + Duration::from_secs(30)
        )
        .err(),
        Some(StagingError::CleanupFailed)
    );
    assert!(!f.parent.join("export-key").exists());
    assert_eq!(
        fs::read(f.parent.join("foreign/keep")).unwrap(),
        b"not ours"
    );
    assert_eq!(
        Lifecycle::begin(&parent, &CheckpointId::parse("next").unwrap(), None).err(),
        Some(StagingError::CleanupRequired)
    );
}
#[test]
fn published_and_partially_shared_original_roots_are_reclaimed_before_unlock() {
    for fully_shared in [false, true] {
        let f = Fixture::new();
        let mut area = f.area(true);
        let id = area.reader.unwrap();
        area.data.readonly().unwrap();
        area.pending.readonly().unwrap();
        area.lifecycle.published = true;
        area.parent
            .publish(&area.pending_name, &area.final_name)
            .unwrap();
        if fully_shared {
            area.pending
                .share_contents(id, Instant::now() + Duration::from_secs(10))
                .unwrap();
            area.pending.activate_reader().unwrap();
        } else {
            area.data
                .share_contents(id, Instant::now() + Duration::from_secs(10))
                .unwrap();
            area.data.activate_reader().unwrap();
        }
        let final_path = f.parent.join(&area.final_name);
        assert!(final_path.exists());
        area.abort().unwrap();
        assert!(!final_path.exists());
        assert_eq!(fs::read_dir(&f.parent).unwrap().count(), 1);
        f.area(true).abort().unwrap();
    }
}
#[test]
fn original_descriptor_never_deletes_a_replacement_directory() {
    let f = Fixture::new();
    let area = f.area(false);
    let old = f.temp.path().join("original-export");
    fs::rename(area.export_root(), &old).unwrap();
    private(area.export_root());
    fs::write(area.export_root().join("keep"), b"foreign").unwrap();
    let replacement = area.export_root().join("keep");
    assert_eq!(area.abort(), Err(StagingError::CleanupFailed));
    assert_eq!(fs::read(replacement).unwrap(), b"foreign");
    let parent = Directory::open(&f.parent, true).unwrap();
    assert_eq!(
        Lifecycle::begin(&parent, &CheckpointId::parse("next").unwrap(), None).err(),
        Some(StagingError::CleanupRequired)
    );
    assert!(old.is_dir());
}
#[test]
fn marker_removal_is_idempotent_after_its_directory_entry_has_already_disappeared() {
    let f = Fixture::new();
    let parent = Directory::open(&f.parent, true).unwrap();
    let mut lifecycle =
        Lifecycle::begin(&parent, &CheckpointId::parse("marker").unwrap(), None).unwrap();
    lifecycle.removing_marker = true;
    lifecycle
        .marker
        .discard_until(
            &parent,
            &lifecycle.name,
            None,
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
    lifecycle
        .finish(
            &parent,
            &mut Budget::until(Instant::now() + Duration::from_secs(30)),
        )
        .unwrap();
    assert!(lifecycle.finished);
}

#[test]
fn expired_freeze_never_starts_another_cleanup_window_and_registered_gc_can_resume() {
    let f = Fixture::new();
    let mut area = f.area(false);
    let export = area.export_root().to_owned();
    fs::write(export.join("canonical/raw"), b"unfinished raw").unwrap();
    area.deadline = Instant::now();
    assert_eq!(area.abort(), Err(StagingError::CleanupFailed));
    assert!(export.exists());
    let result = crate::checkpoint::staging::reclaim_staging(
        &f.parent,
        None,
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(result.interrupted, 1);
    assert!(!export.exists());
}
