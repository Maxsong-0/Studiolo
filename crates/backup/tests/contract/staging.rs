//! 真实文件系统与过滤边界；owner 水位明确使用纯 fixture，不代替七 owner 集成验收。
pub(super) mod fixture;
mod housekeeping;
mod lifecycle;
mod shared;
use fixture::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    time::{Duration, Instant},
};
use studiolo_backup::checkpoint::staging::{
    SealedCheckpoint, SourceCatalog, StagingArea, StagingError,
};
use studiolo_core::{BackupScope, Revision, VaultPath, checkpoint::InstantMillis};

#[test]
fn independent_readonly_view_seals_all_metadata_and_excludes_raw() {
    let f = Fixture::new();
    let area = f.area("same/ID includes path punctuation");
    let full = f.object(&area, "canonical/note.md", b"canonical", BackupScope::Full);
    let mut summary = f.object(
        &area,
        "canonical/raw.txt",
        b"secret raw body",
        BackupScope::SummaryOnly,
    );
    summary.redacted = Some(redacted(b"redacted summary"));
    let none = f.object(
        &area,
        "canonical/none.txt",
        b"must not travel",
        BackupScope::None,
    );
    let versions = area.export_root().join("canonical/.stversions");
    private_dir(&versions);
    private_file(&versions.join("old.txt"), b"must not travel either");
    let original = area.export_root().join("canonical/note.md");
    let original_inode = fs::metadata(&original).unwrap().ino();
    let catalog = SourceCatalog::new([full, summary, none]).unwrap();
    let sealed = area.seal(&catalog, f.inputs()).unwrap();
    let root = &sealed.ticket().transfer_source.root;
    assert_eq!(
        fs::read(root.join("data/canonical/note.md")).unwrap(),
        b"canonical"
    );
    assert_ne!(
        original_inode,
        fs::metadata(root.join("data/canonical/note.md"))
            .unwrap()
            .ino()
    );
    assert!(!original.exists());
    for forbidden in ["raw.txt", "none.txt", ".stversions"] {
        assert!(!root.join("data/canonical").join(forbidden).exists());
    }
    let summaries = fs::read_dir(root.join("data/summaries"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(fs::read(summaries[0].path()).unwrap(), b"redacted summary");
    let files = sealed.inventory().files().collect::<Vec<_>>();
    assert_eq!(files.len(), 5, "完整正文、摘要和三个封存元数据");
    for file in files {
        let bytes = fs::read(root.join(file.path().as_str())).unwrap();
        assert_eq!(file.bytes(), bytes.len() as u64);
        assert_eq!(file.sha256(), redacted(&bytes).sha256);
    }
    assert!(
        sealed
            .inventory()
            .directories()
            .any(|path| path == "data/canonical")
    );
    assert_eq!(
        fs::metadata(root).unwrap().permissions().mode() & 0o777,
        0o500
    );
    assert_eq!(
        fs::metadata(root.join("manifest.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o400
    );
    assert!(
        SealedCheckpoint::open(
            &f.parent,
            sealed.reference(),
            InstantMillis::from_millis(1100),
            deadline()
        )
        .is_ok()
    );
    let mut wrong = sealed.reference().clone();
    wrong.seal_sha256 = "0".repeat(64);
    assert!(
        SealedCheckpoint::open(
            &f.parent,
            &wrong,
            InstantMillis::from_millis(1100),
            deadline()
        )
        .is_err()
    );
    assert!(f.try_area("same/ID includes path punctuation").is_err());
}

#[test]
fn missing_extra_stale_or_unlabelled_registration_rejects_whole_checkpoint() {
    for mode in 0..5 {
        let f = Fixture::new();
        let area = f.area("invalid");
        let mut record = f.object(&area, "canonical/note.md", b"current", BackupScope::Full);
        match mode {
            0 => record.source.labels = None,
            1 => {
                record.source.declared_revision =
                    Some(Revision::compute(f.vault, &record.source.path, b"old"))
            }
            2 => record.export_path = VaultPath::parse("canonical/missing.md").unwrap(),
            3 => record.source.declared_revision = None,
            _ => private_file(&area.export_root().join("canonical/unregistered"), b"raw"),
        }
        let reference_path = SealedCheckpoint::root_path(&f.parent, &checkpoint("invalid"));
        assert!(
            area.seal(&SourceCatalog::new([record]).unwrap(), f.inputs())
                .is_err()
        );
        assert!(!reference_path.exists());
    }
}

#[test]
fn alias_overlap_link_hardlink_expiry_and_unsafe_permissions_fail_closed() {
    let f = Fixture::new();
    assert!(
        StagingArea::create(
            &f.parent,
            checkpoint("overlap"),
            std::slice::from_ref(&f.parent),
            &f.ephemeral,
            deadline()
        )
        .is_err()
    );
    assert!(
        StagingArea::create(
            &f.parent,
            checkpoint("ephemeral"),
            std::slice::from_ref(&f.source),
            &f.source,
            deadline()
        )
        .is_err()
    );
    let link = f.temp.path().join("link");
    symlink(&f.parent, &link).unwrap();
    assert!(
        StagingArea::create(
            &link,
            checkpoint("link"),
            std::slice::from_ref(&f.source),
            &f.ephemeral,
            deadline()
        )
        .is_err()
    );
    for mode in 0..5 {
        let area = f.area(&format!("unsafe-{mode}"));
        let record = f.object(&area, "canonical/note", b"data", BackupScope::Full);
        let file = area.export_root().join("canonical/note");
        let mut inputs = f.inputs();
        match mode {
            0 => fs::hard_link(&file, area.export_root().join("canonical/alias")).unwrap(),
            1 => {
                fs::remove_file(&file).unwrap();
                symlink(f.source.join("outside"), &file).unwrap();
            }
            2 => fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap(),
            3 => inputs.expires_at = InstantMillis::from_millis(12000),
            _ => inputs.expires_at = inputs.now,
        }
        assert!(
            area.seal(&SourceCatalog::new([record]).unwrap(), inputs)
                .is_err()
        );
    }
    assert!(
        StagingArea::create(
            &f.parent,
            checkpoint("timeout"),
            std::slice::from_ref(&f.source),
            &f.ephemeral,
            Instant::now() - Duration::from_secs(1)
        )
        .is_err()
    );
}

#[test]
fn post_seal_content_permission_extra_file_and_expiry_changes_are_detected() {
    for mode in 0..4 {
        let f = Fixture::new();
        let area = f.area("tamper");
        let record = f.object(&area, "canonical/note", b"data", BackupScope::Full);
        let sealed = area
            .seal(&SourceCatalog::new([record]).unwrap(), f.inputs())
            .unwrap();
        let root = &sealed.ticket().transfer_source.root;
        let mut now = InstantMillis::from_millis(1100);
        match mode {
            0 => {
                let file = root.join("data/canonical/note");
                fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&file, b"evil").unwrap();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o400)).unwrap();
            }
            1 => fs::set_permissions(
                root.join("data/canonical/note"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap(),
            2 => {
                fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
                private_file(&root.join("extra"), b"x");
                fs::set_permissions(root, fs::Permissions::from_mode(0o500)).unwrap();
            }
            _ => now = InstantMillis::from_millis(10000),
        }
        assert!(SealedCheckpoint::open(&f.parent, sealed.reference(), now, deadline()).is_err());
    }
}

#[test]
fn summary_requires_verified_redaction_and_none_never_accepts_an_extra_body() {
    for mode in 0..4 {
        let f = Fixture::new();
        let area = f.area("summary");
        let mut record = f.object(
            &area,
            "canonical/raw",
            b"raw",
            if mode == 3 {
                BackupScope::None
            } else {
                BackupScope::SummaryOnly
            },
        );
        if mode != 0 {
            record.redacted = Some(redacted(b"safe"));
        }
        if mode == 1 {
            record.redacted.as_mut().unwrap().sha256 = "0".repeat(64);
        }
        if mode == 2 {
            record.source.expires_at = None;
        }
        assert!(matches!(
            area.seal(&SourceCatalog::new([record]).unwrap(), f.inputs()),
            Err(StagingError::Catalog)
        ));
    }
}
