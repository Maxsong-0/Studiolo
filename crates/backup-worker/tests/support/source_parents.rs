//! 单 UID 箱只检验目录逐级规则；不冒充已完成生产跨 UID 读取。
use super::*;
use std::{
    fs as stdfs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    time::Duration,
};
fn mode(path: &Path, mode: u32) {
    stdfs::set_permissions(path, stdfs::Permissions::from_mode(mode)).unwrap();
}
fn read(base: &Path) -> Result<Parents> {
    let fd = fs::open(base, FLAGS, Mode::empty()).unwrap();
    let stat = fs::fstat(&fd).unwrap();
    Parents::descend(
        fd,
        &[
            OsStr::new("ancestor"),
            OsStr::new("parent"),
            OsStr::new("sealed"),
        ],
        stat.st_uid,
        stat.st_gid,
        Instant::now() + Duration::from_secs(10),
    )
}
#[test]
fn original_ancestor_identity_and_readonly_root_are_fixed() {
    let temp = tempfile::tempdir().unwrap();
    let ancestor = temp.path().join("ancestor");
    let parent = ancestor.join("parent");
    let root = parent.join("sealed");
    stdfs::create_dir_all(&root).unwrap();
    mode(temp.path(), 0o700);
    mode(&ancestor, 0o700);
    mode(&parent, 0o750);
    mode(&root, 0o550);
    let original = read(temp.path()).unwrap();
    assert_eq!(read(temp.path()).unwrap(), original);
    stdfs::rename(&ancestor, temp.path().join("old")).unwrap();
    stdfs::create_dir(&ancestor).unwrap();
    mode(&ancestor, 0o700);
    stdfs::rename(temp.path().join("old/parent"), &parent).unwrap();
    assert_ne!(
        read(temp.path()).unwrap(),
        original,
        "末端相同也拒绝祖先替换"
    );
    mode(&ancestor, 0o770);
    assert!(read(temp.path()).is_err());
    mode(&ancestor, 0o700);
    mode(&root, 0o750);
    assert!(read(temp.path()).is_err());
    mode(&root, 0o550);
    stdfs::rename(&root, parent.join("old")).unwrap();
    symlink("old", &root).unwrap();
    assert!(read(temp.path()).is_err());
}
#[test]
fn foreign_writer_group_write_special_bits_acl_and_unbounded_path_refuse() {
    let temp = tempfile::tempdir().unwrap();
    let fd = fs::open(temp.path(), FLAGS, Mode::empty()).unwrap();
    let mut stat = fs::fstat(&fd).unwrap();
    let producer = stat.st_uid;
    for mode in [0o777, 0o770, 0o1700, 0o2700, 0o4700] {
        stat.st_mode = mode;
        assert_eq!(validate(&stat, producer), Err(ProcessError::Source));
    }
    stat.st_mode = 0o700;
    stat.st_uid = producer + 1;
    assert_eq!(validate(&stat, producer), Err(ProcessError::Source));
    let uid = stdfs::metadata(temp.path()).unwrap().uid();
    let mut bytes = 2_u32.to_le_bytes().to_vec();
    for (tag, permissions, id) in [
        (1_u16, 7_u16, u32::MAX),
        (2, 4, uid),
        (4, 0, u32::MAX),
        (16, 4, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(permissions.to_le_bytes());
        bytes.extend(id.to_le_bytes());
    }
    fs::fsetxattr(
        &fd,
        "system.posix_acl_access",
        &bytes,
        fs::XattrFlags::empty(),
    )
    .unwrap();
    assert!(pin(&fd, producer).is_err());
    assert!(
        Parents::read(
            Path::new("relative/path"),
            producer,
            stat.st_gid,
            Instant::now()
        )
        .is_err()
    );
    assert!(
        Parents::descend(
            fd,
            &vec![OsStr::new("a"); 65],
            producer,
            stat.st_gid,
            Instant::now()
        )
        .is_err()
    );
}
