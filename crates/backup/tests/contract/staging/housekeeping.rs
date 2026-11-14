//! 回收只在临时测试根运行；到期时间为显式夹具，不冒充真实生产定时器。
use super::{fixture::*, *};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_backup::checkpoint::staging::{CleanupSummary, reclaim_staging};
fn retained(f: &Fixture) -> PathBuf {
    fs::read_dir(&f.parent)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("retained-")
        })
        .unwrap()
}
fn sealed(f: &Fixture, shared: bool, unexpired: bool) -> (SealedCheckpoint, Option<u32>) {
    let group = if shared {
        fs::set_permissions(&f.parent, fs::Permissions::from_mode(0o750)).unwrap();
        Some(fs::metadata(&f.parent).unwrap().gid())
    } else {
        None
    };
    let area = if let Some(gid) = group {
        StagingArea::create_for_reader(
            &f.parent,
            checkpoint("gc"),
            std::slice::from_ref(&f.source),
            &f.ephemeral,
            gid,
            deadline(),
        )
        .unwrap()
    } else {
        f.area("gc")
    };
    let mut record = f.object(
        &area,
        "canonical/note",
        b"retained bytes",
        BackupScope::Full,
    );
    let mut inputs = f.inputs();
    if unexpired {
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        )
        .unwrap();
        inputs.now = InstantMillis::from_millis(now);
        inputs.content_cutoff = InstantMillis::from_millis(now - 1);
        inputs.expires_at = InstantMillis::from_millis(now + 60_000);
        record.source.expires_at = Some(inputs.expires_at);
    }
    (
        area.seal(&SourceCatalog::new([record]).unwrap(), inputs)
            .unwrap(),
        group,
    )
}
fn replace_json(path: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut value);
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
}
#[test]
fn expired_private_and_shared_views_are_removed_with_their_original_receipts() {
    for shared in [false, true] {
        let f = Fixture::new();
        let (sealed, group) = sealed(&f, shared, false);
        let marker = retained(&f);
        assert_eq!(fs::read_dir(&marker).unwrap().count(), 4);
        let root = sealed.ticket().transfer_source.root.clone();
        assert_eq!(
            reclaim_staging(&f.parent, group, deadline()).unwrap(),
            CleanupSummary {
                expired: 1,
                ..CleanupSummary::default()
            }
        );
        assert!(!root.exists());
        assert!(!marker.exists());
        assert_eq!(fs::read_dir(&f.parent).unwrap().count(), 1);
        assert_eq!(
            reclaim_staging(&f.parent, group, deadline()).unwrap(),
            CleanupSummary::default()
        );
    }
}
#[test]
fn unexpired_source_remains_readable_and_reports_its_exact_next_deadline() {
    let f = Fixture::new();
    let (sealed, group) = sealed(&f, true, true);
    let summary = reclaim_staging(&f.parent, group, deadline()).unwrap();
    assert_eq!(summary.retained, 1);
    assert_eq!(summary.expired, 0);
    assert_eq!(
        summary.next_expiry,
        Some(sealed.ticket().transfer_source.expires_at)
    );
    assert_eq!(
        fs::read(
            sealed
                .ticket()
                .transfer_source
                .root
                .join("data/canonical/note")
        )
        .unwrap(),
        b"retained bytes"
    );
}
#[test]
fn corrupt_registration_or_replaced_root_never_grants_deletion() {
    for mode in 0..6 {
        let f = Fixture::new();
        let (sealed, group) = sealed(&f, false, false);
        let marker = retained(&f);
        let root = &sealed.ticket().transfer_source.root;
        match mode {
            0 => replace_json(&marker.join("pending.json"), |v| {
                v["directory"][1] = serde_json::json!(0)
            }),
            1 => replace_json(&marker.join("identity.json"), |v| {
                v["parent"][1] = serde_json::json!(0)
            }),
            2 => replace_json(&marker.join("sealed.json"), |v| {
                let mut seal: serde_json::Value =
                    serde_json::from_str(v["seal"].as_str().unwrap()).unwrap();
                seal["ticket"]["transfer_source"]["expires_at"] = serde_json::json!(0);
                v["seal"] = serde_json::json!(serde_json::to_string(&seal).unwrap());
            }),
            3 => {
                fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
                fs::rename(root, f.temp.path().join("original-sealed")).unwrap();
                private_dir(root);
                private_file(&root.join("keep"), b"foreign");
            }
            4 => private_file(&marker.join("unknown"), b"do not touch"),
            _ => {
                private_dir(&marker.join("sealed.new"));
                private_file(&marker.join("sealed.new/keep"), b"foreign directory");
            }
        }
        assert!(
            reclaim_staging(&f.parent, group, deadline()).is_err(),
            "mode={mode}"
        );
        assert!(root.exists());
        assert!(marker.exists());
        if mode == 3 {
            assert_eq!(fs::read(root.join("keep")).unwrap(), b"foreign");
        } else {
            assert_eq!(
                fs::read(root.join("data/canonical/note")).unwrap(),
                b"retained bytes"
            );
        }
    }
}

#[test]
fn interrupted_expiry_cleanup_resumes_without_requiring_already_removed_seal_bytes() {
    let f = Fixture::new();
    let (sealed, group) = sealed(&f, true, false);
    let root = &sealed.ticket().transfer_source.root;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_file(root.join("seal.json")).unwrap();
    let result = reclaim_staging(&f.parent, group, deadline()).unwrap();
    assert_eq!(result.expired, 1);
    assert!(!root.exists());
    assert_eq!(fs::read_dir(&f.parent).unwrap().count(), 1);
}
#[test]
fn missing_lease_or_unregistered_orphan_does_not_initialize_or_partially_delete() {
    let f = Fixture::new();
    assert!(reclaim_staging(&f.parent, None, deadline()).is_err());
    assert_eq!(fs::read_dir(&f.parent).unwrap().count(), 0);
    let (sealed, group) = sealed(&f, false, false);
    private_dir(&f.parent.join("export-legacy"));
    private_file(&f.parent.join("export-legacy/raw"), b"legacy");
    assert_eq!(
        reclaim_staging(&f.parent, group, deadline()),
        Err(StagingError::CleanupRequired)
    );
    assert!(
        sealed.ticket().transfer_source.root.exists(),
        "先完整核对，不能先删一半才发现未知目录"
    );
    assert!(retained(&f).exists());
}
