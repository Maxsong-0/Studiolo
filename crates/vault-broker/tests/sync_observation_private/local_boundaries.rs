//! 新入口沿用私有 reader 的属主/模式与无链接语义；没有正文或根身份扩权。
use super::*;
use crate::{LocalReadPresence, ReadOnlyVault};
use std::os::unix::fs::{PermissionsExt, symlink};

fn mode(path: &std::path::Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}
fn private_fixture() -> Fixture {
    let f = Fixture::new();
    f.write("notes/a", b"private");
    mode(&f.root, 0o700);
    mode(&f.root.join("notes"), 0o700);
    mode(&f.root.join("notes/a"), 0o600);
    f
}

#[test]
fn private_modes_are_checked_at_begin_finish_and_final_revalidation() {
    for target in ["", "notes", "notes/a"] {
        let f = private_fixture();
        let reader = ReadOnlyVault::open_private(&f.root).unwrap();
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        mode(
            &f.root.join(target),
            if target == "notes/a" { 0o640 } else { 0o750 },
        );
        reset();
        assert!(matches!(
            reader.finish_scoped_observation(survey, &[]),
            Err(BrokerError::Io {
                operation: UNSAFE_MODE,
                ..
            })
        ));
        assert_eq!(count(1), 0);
        assert!(matches!(
            reader.begin_scoped_observation(&selection(), &[]),
            Err(BrokerError::Io {
                operation: UNSAFE_MODE,
                ..
            })
        ));
        mode(
            &f.root.join(target),
            if target == "notes/a" { 0o600 } else { 0o700 },
        );
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        let result = reader.finish_scoped_observation(survey, &[]).unwrap();
        mode(
            &f.root.join(target),
            if target == "notes/a" { 0o644 } else { 0o755 },
        );
        assert!(matches!(
            reader.revalidate_scoped_observation(&result),
            Err(BrokerError::Io {
                operation: UNSAFE_MODE,
                ..
            })
        ));
    }
}

#[test]
fn private_modes_remain_checked_during_body_and_inherited_child_reader() {
    for phase in [1, 3] {
        let f = private_fixture();
        let reader = ReadOnlyVault::open_private(&f.root).unwrap();
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        let target = f.root.join("notes/a");
        let _fault = fault(phase, move || mode(&target, 0o644));
        assert!(matches!(
            reader.finish_scoped_observation(survey, &[]),
            Err(BrokerError::Io {
                operation: UNSAFE_MODE,
                ..
            })
        ));
        assert!(count(phase) > 0);
    }
    let f = private_fixture();
    let reader = ReadOnlyVault::open_private(&f.root).unwrap();
    let child = reader.open_directory(&path("notes")).unwrap();
    let selection = SyncReadSelection {
        folders: vec![],
        files: vec![SyncReadRule {
            path: path("a"),
            max_bytes: 20,
        }],
        excluded_prefixes: vec![],
    };
    let survey = child.begin_scoped_observation(&selection, &[]).unwrap();
    mode(&f.root.join("notes/a"), 0o644);
    assert!(matches!(
        child.finish_scoped_observation(survey, &[]),
        Err(BrokerError::Io {
            operation: UNSAFE_MODE,
            ..
        })
    ));
}

#[test]
fn excluded_unsafe_subtree_is_neither_opened_nor_read_by_private_observation() {
    let f = private_fixture();
    f.write("notes/private/raw", b"excluded");
    mode(&f.root.join("notes/private"), 0o777);
    symlink(
        "/definitely-missing-local-observation-target",
        f.root.join("notes/generated.md"),
    )
    .unwrap();
    symlink(
        "/definitely-missing-local-observation-target",
        f.root.join("notes/.hidden"),
    )
    .unwrap();
    f.write("notes/TASKS.md", b"handwritten task remains");
    let reader = ReadOnlyVault::open_private(&f.root).unwrap();
    reset();
    let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    assert_eq!(survey.paths().len(), 1);
    assert_eq!((count(1), count(3)), (0, 0));
    assert_eq!(count(4), 1, "只进入 notes，禁止下探排除目录");
    let result = reader.finish_scoped_observation(survey, &[]).unwrap();
    assert_eq!(result.entries().len(), 1);
    assert_eq!((count(1), count(3)), (1, 1));
    assert_eq!(
        fs::read(f.root.join("notes/TASKS.md")).unwrap(),
        b"handwritten task remains"
    );
}

#[test]
fn local_selected_symlink_hardlink_and_fifo_fail_before_body() {
    for attack in 0..3 {
        let f = Fixture::new();
        f.write("notes/a", b"original");
        let target = f.root.join("notes/a");
        match attack {
            0 => {
                fs::remove_file(&target).unwrap();
                symlink("/etc/passwd", &target).unwrap();
            }
            1 => fs::hard_link(&target, f.root.join("notes/b")).unwrap(),
            _ => {
                fs::remove_file(&target).unwrap();
                assert!(
                    std::process::Command::new("/usr/bin/mkfifo")
                        .arg(&target)
                        .status()
                        .unwrap()
                        .success()
                );
            }
        }
        let reader = ReadOnlyVault::open(&f.root).unwrap();
        reset();
        // 链接按链接拒绝；硬链接与 FIFO 都不是唯一普通文件。硬链接两个名字谁先被枚举
        // 取决于目录顺序，只断言拒绝原因。
        let expected = if attack == 0 {
            "linked or cross-device sync object"
        } else {
            "sync observation requires unique regular file"
        };
        let result = reader.begin_scoped_observation(&selection(), &[]);
        assert!(
            matches!(&result, Err(BrokerError::UnsafeNode { reason, .. }) if *reason == expected),
            "attack {attack}: {:?}",
            result.err()
        );
        assert_eq!(count(1), 0);
    }
}

#[test]
fn local_surveys_and_final_proofs_reject_other_reader_or_reopened_instance() {
    let f = Fixture::new();
    f.write("notes/a", b"same");
    let original = ReadOnlyVault::open(&f.root).unwrap();
    let different = ReadOnlyVault::open(&f.root).unwrap();
    let survey = original
        .begin_scoped_observation(&selection(), &[])
        .unwrap();
    reset();
    assert!(matches!(
        different.finish_scoped_observation(survey, &[]),
        Err(BrokerError::CaptureChanged)
    ));
    assert_eq!(count(1), 0);
    let survey = original
        .begin_scoped_observation(&selection(), &[])
        .unwrap();
    let proof = original.finish_scoped_observation(survey, &[]).unwrap();
    let unfinished = original
        .begin_scoped_observation(&selection(), &[])
        .unwrap();
    drop(original);
    let reopened = ReadOnlyVault::open(&f.root).unwrap();
    assert!(matches!(
        reopened.finish_scoped_observation(unfinished, &[]),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(matches!(
        reopened.revalidate_scoped_observation(&proof),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(matches!(
        different.revalidate_scoped_observation(&proof),
        Err(BrokerError::CaptureChanged)
    ));
}

#[test]
fn pinned_root_semantics_remain_explicit_after_ambient_path_moves() {
    let f = Fixture::new();
    f.write("notes/a", b"old root");
    let reader = ReadOnlyVault::open(&f.root).unwrap();
    fs::rename(&f.root, f.root.with_extension("retained")).unwrap();
    f.write("notes/a", b"replacement root");
    // 调查始于移动之后；固定 FD 仍观察旧目录，不声称宿主路径仍绑定。
    let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    let proof = reader.finish_scoped_observation(survey, &[]).unwrap();
    let LocalReadPresence::Present(file) = &proof.entries()[&path("notes/a")] else {
        panic!("应存在");
    };
    assert_eq!(file.identity.bytes, 8);
    reader.revalidate_scoped_observation(&proof).unwrap();
    assert_eq!(
        fs::read(f.root.join("notes/a")).unwrap(),
        b"replacement root"
    );
}
