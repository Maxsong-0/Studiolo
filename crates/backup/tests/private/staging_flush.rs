//! 拷大文件时没落盘的量封顶（BUG-054）：每写满 64 MiB 落一次盘，不等 `finish`。
use super::*;
use std::{fs as files, os::unix::fs::PermissionsExt};

const MIB: usize = 1 << 20;

fn parent() -> (tempfile::TempDir, Directory) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("parent");
    files::create_dir(&path).unwrap();
    files::set_permissions(&path, files::Permissions::from_mode(0o700)).unwrap();
    let parent = Directory::open(&path, true).unwrap();
    (temp, parent)
}

#[test]
fn a_large_file_flushes_every_64_mib_while_it_is_written() {
    // 写的步长和封存拷贝的缓冲一样是 256 KiB；不到一段一次都不另落，刚满一段、过两段各落在段界上。
    let chunk = [7_u8; 256 * 1024];
    for (mebibytes, expected) in [
        (1, vec![]),
        (63, vec![]),
        (64, vec![64]),
        (130, vec![64, 128]),
    ] {
        let (temp, parent) = parent();
        FLUSHED.take();
        let mut output =
            Regular::create(&parent, "audit.sqlite3", (mebibytes * MIB) as u64).unwrap();
        for _ in 0..mebibytes * MIB / chunk.len() {
            output.write(&chunk).unwrap();
        }
        let lengths = FLUSHED.take();
        output.finish().unwrap();
        let expected = expected
            .into_iter()
            .map(|at: usize| (at * MIB) as u64)
            .collect::<Vec<_>>();
        assert_eq!(lengths, expected, "{mebibytes} MiB");
        // `finish` 照旧再落一次、改成只读。
        let mode = files::metadata(temp.path().join("parent/audit.sqlite3"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o400);
    }
}

/// 一个文件一份计数：小文件加起来过了一段也不另落，落盘只跟单个文件的大小有关。
#[test]
fn many_small_files_do_not_flush_early() {
    let (_temp, parent) = parent();
    FLUSHED.take();
    let chunk = vec![7_u8; MIB];
    for index in 0..70 {
        let mut output = Regular::create(&parent, &format!("note-{index}.md"), MIB as u64).unwrap();
        output.write(&chunk).unwrap();
        output.finish().unwrap();
    }
    assert_eq!(FLUSHED.take(), Vec::<u64>::new());
}
