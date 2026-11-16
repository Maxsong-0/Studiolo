//! 仅虚构文本；普通测试目录必须被生产只读 ramfs 检查拒绝。
use super::*;
use std::{fs as stdfs, os::unix::fs::PermissionsExt, time::Duration};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn read(path: &std::path::Path, slot: Slot) -> Result<(Zeroizing<String>, Metadata)> {
    let file = File::open(path).unwrap();
    read_checked(&file, file.metadata().unwrap().uid(), slot, deadline())
}
#[test]
fn fixed_slots_and_bounded_secret_formats_have_no_silent_normalization() {
    assert_eq!(Slot::Repository.name(), "repository-password");
    assert_eq!(Slot::Username.name(), "rest-username");
    assert_eq!(Slot::Password.name(), "rest-password");
    for slot in [Slot::Repository, Slot::Username, Slot::Password] {
        assert_eq!(parse(b"fixture\n", slot).unwrap().as_str(), "fixture");
        for bytes in [
            b"".as_slice(),
            b"\n",
            b"\0",
            b"a\nb",
            b"a\r\n",
            b"a\n\n",
            b" a",
            b"a ",
            &[0xff],
        ] {
            assert!(parse(bytes, slot).is_err());
        }
        assert!(parse(&vec![b'a'; slot.limit()], slot).is_ok());
        assert!(parse(&vec![b'a'; slot.limit() + 1], slot).is_err());
    }
    assert!(parse(b"name:extra", Slot::Username).is_err());
    assert!(parse(b"fixture:password", Slot::Password).is_ok());
}
#[test]
fn private_file_read_rejects_permissions_aliases_budget_and_changed_original() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("credential");
    let write = |bytes: &[u8], mode| {
        if path.exists() {
            stdfs::set_permissions(&path, stdfs::Permissions::from_mode(0o600)).unwrap();
        }
        stdfs::write(&path, bytes).unwrap();
        stdfs::set_permissions(&path, stdfs::Permissions::from_mode(mode)).unwrap();
    };
    write(b"fictional\n", 0o400);
    let (value, pin) = read(&path, Slot::Password).unwrap();
    assert_eq!(value.as_str(), "fictional");
    let file = File::open(&path).unwrap();
    assert!(
        super::super::root::readonly_memory(&file).is_err(),
        "普通磁盘文件不能冒充凭据内存挂载"
    );
    assert!(read_checked(&file, pin.uid(), Slot::Password, Instant::now()).is_err());
    assert!(read_checked(&file, pin.uid() + 1, Slot::Password, deadline()).is_err());
    for mode in [0o600, 0o440, 0o404, 0o450, 0o1400] {
        write(b"fictional", mode);
        assert!(read(&path, Slot::Password).is_err());
    }
    write(&vec![b'a'; 4098], 0o400);
    assert!(read(&path, Slot::Password).is_err());
    write(b"", 0o400);
    assert!(read(&path, Slot::Password).is_err());
    write(b"fictional", 0o400);
    stdfs::hard_link(&path, temp.path().join("alias")).unwrap();
    assert!(read(&path, Slot::Password).is_err());
    stdfs::remove_file(temp.path().join("alias")).unwrap();
    let (_, original) = read(&path, Slot::Password).unwrap();
    write(b"different", 0o400);
    assert!(!same(&original, &stdfs::metadata(&path).unwrap()));
    let (_, original) = read(&path, Slot::Password).unwrap();
    stdfs::rename(&path, temp.path().join("old")).unwrap();
    write(b"different", 0o400);
    assert!(!same(&original, &stdfs::metadata(&path).unwrap()));
}
