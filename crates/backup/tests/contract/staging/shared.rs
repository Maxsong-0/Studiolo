//! 真文件交接契约；当前测试身份不代表已演练生产的独立系统账号。
use super::{fixture::*, *};
use studiolo_backup::checkpoint::staging::ReaderIdentity;

fn group(f: &Fixture) -> u32 {
    let gid = fs::metadata(&f.parent).unwrap().gid();
    fs::set_permissions(&f.parent, fs::Permissions::from_mode(0o750)).unwrap();
    gid
}
fn area(f: &Fixture, gid: u32) -> StagingArea {
    StagingArea::create_for_reader(
        &f.parent,
        checkpoint("shared"),
        std::slice::from_ref(&f.source),
        &f.ephemeral,
        gid,
        deadline(),
    )
    .unwrap()
}
fn mode(path: &std::path::Path) -> u32 {
    fs::metadata(path).unwrap().mode() & 0o7777
}
#[test]
fn only_verified_filtered_seal_is_granted_to_configured_group() {
    let f = Fixture::new();
    let gid = group(&f);
    let area = area(&f, gid);
    let export = area.export_root().to_owned();
    assert_eq!(mode(&export), 0o700);
    assert_eq!(mode(&export.join("canonical")), 0o700);
    let full = f.object(&area, "canonical/note", b"allowed", BackupScope::Full);
    let none = f.object(&area, "canonical/raw", b"private raw", BackupScope::None);
    let sealed = area
        .seal(&SourceCatalog::new([full, none]).unwrap(), f.inputs())
        .unwrap();
    assert!(!export.exists());
    let root = &sealed.ticket().transfer_source.root;
    assert_eq!(mode(root), 0o550);
    assert_eq!(mode(&root.join("data/canonical")), 0o550);
    assert_eq!(mode(&root.join("data/canonical/note")), 0o440);
    assert_eq!(mode(&root.join("manifest.json")), 0o440);
    assert_eq!(fs::metadata(root).unwrap().gid(), gid);
    assert!(!root.join("data/canonical/raw").exists());
    let now = InstantMillis::from_millis(1100);
    assert!(
        SealedCheckpoint::open_shared(&f.parent, sealed.reference(), gid, now, deadline()).is_ok()
    );
    assert!(SealedCheckpoint::open(&f.parent, sealed.reference(), now, deadline()).is_err());
    let identity = ReaderIdentity::from_config(fs::metadata(root).unwrap().uid(), gid).unwrap();
    assert!(
        SealedCheckpoint::open_for_worker(&f.parent, sealed.reference(), identity, now, deadline())
            .is_err(),
        "同 UID 不能冒充独立 worker"
    );
    let wrong = ReaderIdentity::from_config(fs::metadata(root).unwrap().uid() + 1, gid).unwrap();
    assert!(
        SealedCheckpoint::open_for_worker(&f.parent, sealed.reference(), wrong, now, deadline())
            .is_err(),
        "不同 UID 也不能把错误的生产者当成授权"
    );
    assert!(
        SealedCheckpoint::open_shared(&f.parent, sealed.reference(), gid + 1, now, deadline())
            .is_err()
    );
}

#[test]
fn shared_parent_and_children_reject_wrong_mode_identity_content_and_acl() {
    for change in 0..8 {
        let f = Fixture::new();
        let gid = group(&f);
        let area = area(&f, gid);
        let record = f.object(&area, "canonical/note", b"allowed", BackupScope::Full);
        let sealed = area
            .seal(&SourceCatalog::new([record]).unwrap(), f.inputs())
            .unwrap();
        let root = &sealed.ticket().transfer_source.root;
        let file = root.join("data/canonical/note");
        match change {
            0 => fs::set_permissions(&f.parent, fs::Permissions::from_mode(0o770)).unwrap(),
            1 => fs::set_permissions(root, fs::Permissions::from_mode(0o570)).unwrap(),
            2 => fs::set_permissions(&file, fs::Permissions::from_mode(0o460)).unwrap(),
            3 => fs::set_permissions(root.join("data"), fs::Permissions::from_mode(0o555)).unwrap(),
            4 => {
                fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
                fs::write(&file, b"changed").unwrap();
                fs::set_permissions(&file, fs::Permissions::from_mode(0o440)).unwrap();
            }
            5 => add_acl(&file, "system.posix_acl_access", 4, 4),
            6 => add_acl(&f.parent, "system.posix_acl_default", 7, 5),
            _ => {
                fs::set_permissions(root, fs::Permissions::from_mode(0o750)).unwrap();
                private_file(&root.join("extra"), b"unexpected");
                fs::set_permissions(root, fs::Permissions::from_mode(0o550)).unwrap();
            }
        }
        assert!(
            SealedCheckpoint::open_shared(
                &f.parent,
                sealed.reference(),
                gid,
                InstantMillis::from_millis(1100),
                deadline()
            )
            .is_err(),
            "change={change}"
        );
    }
}

#[test]
fn publisher_rejects_default_acl_wrong_parent_and_failed_catalog_stays_private() {
    for change in 0..4 {
        let f = Fixture::new();
        let gid = group(&f);
        match change {
            0 => fs::set_permissions(&f.parent, fs::Permissions::from_mode(0o755)).unwrap(),
            1 => add_acl(&f.parent, "system.posix_acl_default", 7, 5),
            2 => {}
            _ => {
                let area = area(&f, gid);
                let export = area.export_root().to_owned();
                let _ = f.object(&area, "canonical/raw", b"unregistered", BackupScope::None);
                assert!(
                    area.seal(&SourceCatalog::new([]).unwrap(), f.inputs())
                        .is_err()
                );
                assert!(!export.exists());
                assert!(!SealedCheckpoint::root_path(&f.parent, &checkpoint("shared")).exists());
                continue;
            }
        }
        assert!(
            StagingArea::create_for_reader(
                &f.parent,
                checkpoint("shared"),
                std::slice::from_ref(&f.source),
                &f.ephemeral,
                if change == 2 { gid + 1 } else { gid },
                deadline()
            )
            .is_err()
        );
    }
    for (uid, gid) in [(0, 1), (1, 0), (u32::MAX, 1), (1, u32::MAX)] {
        assert!(ReaderIdentity::from_config(uid, gid).is_err());
    }
}

fn add_acl(path: &std::path::Path, name: &str, owner: u16, group: u16) {
    // 测试箱只映射当前 UID；用有效的命名条目测试 ACL 拒绝，不虚构第二个系统身份。
    let uid = fs::metadata(path).unwrap().uid();
    let mut bytes = 2_u32.to_le_bytes().to_vec();
    for (tag, permissions, id) in [
        (1_u16, owner, u32::MAX),
        (2, 4, uid),
        (4, group, u32::MAX),
        (16, group, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(permissions.to_le_bytes());
        bytes.extend(id.to_le_bytes());
    }
    rustix::fs::setxattr(path, name, &bytes, rustix::fs::XattrFlags::empty()).unwrap();
}
