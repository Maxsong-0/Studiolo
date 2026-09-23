//! Canonical revision compatibility and visible-file security through the public handle.

use std::{
    fs,
    os::unix::{fs::symlink, net::UnixListener},
};
use studiolo_core::{PathSyntaxError, VaultPath};
use studiolo_vault_broker::{BrokerError, ReadOnlyVault, VaultReader as _};

use super::support::{Fixture, create_fifo, inventory, path};

#[test]
fn revisions_match_existing_broker_and_are_stable_across_reopen() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("a.md"), b"same bytes").unwrap();
    let logical = path("a.md");
    let first = fixture.reader().read_file(&logical, 64).unwrap();
    let second = fixture.reader().read_file(&logical, 64).unwrap();
    let existing = fixture.broker.read(&logical).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.vault_uuid, fixture.broker.vault_uuid());
    assert_eq!(first.path, existing.path);
    assert_eq!(first.content, existing.content);
    assert_eq!(first.revision, existing.revision);
    let replica = ReadOnlyVault::open(&fixture.root)
        .unwrap()
        .read_file(&logical, 64)
        .unwrap();
    assert_ne!(first.revision, replica.revision);
}

#[test]
fn equal_content_in_another_path_or_vault_has_a_different_revision() {
    let first = Fixture::new();
    let second = Fixture::new();
    for (root, name) in [
        (&first.root, "a.md"),
        (&first.root, "b.md"),
        (&second.root, "a.md"),
    ] {
        fs::write(root.join(name), b"same bytes").unwrap();
    }
    let a = first.reader().read_file(&path("a.md"), 64).unwrap();
    let b = first.reader().read_file(&path("b.md"), 64).unwrap();
    let other = second.reader().read_file(&path("a.md"), 64).unwrap();
    assert_ne!(a.revision, b.revision);
    assert_ne!(a.revision, other.revision);
    assert_ne!(a.vault_uuid, other.vault_uuid);
}

#[test]
fn external_changes_are_read_from_disk_without_an_index_refresh() {
    let fixture = Fixture::new();
    let file = fixture.root.join("note.md");
    fs::write(&file, b"before").unwrap();
    let reader = fixture.reader();
    let before = reader.read_file(&path("note.md"), 64).unwrap();
    fs::write(&file, b"after!").unwrap();
    let after = reader.read_file(&path("note.md"), 64).unwrap();
    assert_eq!(after.content, b"after!");
    assert_ne!(before.revision, after.revision);
    assert_eq!(
        after.revision,
        fixture.broker.read(&path("note.md")).unwrap().revision
    );
}

#[test]
fn exact_and_zero_byte_limits_work_and_oversized_reads_fail_structurally() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("note.md"), b"1234").unwrap();
    fs::write(fixture.root.join("empty.md"), []).unwrap();
    let reader = fixture.reader();
    assert_eq!(
        reader.read_file(&path("note.md"), 4).unwrap().content,
        b"1234"
    );
    assert!(
        reader
            .read_file(&path("empty.md"), 0)
            .unwrap()
            .content
            .is_empty()
    );
    let before = inventory(fixture.directory.path());
    for maximum in [0, 3] {
        assert!(matches!(reader.read_file(&path("note.md"), maximum),
            Err(BrokerError::ReadLimitExceeded { path: failed, max_bytes })
                if failed == path("note.md") && max_bytes == maximum));
    }
    assert_eq!(inventory(fixture.directory.path()), before);
}

#[test]
fn hidden_files_links_directories_and_special_nodes_are_rejected() {
    let fixture = Fixture::new();
    let outside = fixture.directory.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.md"), b"outside secret").unwrap();
    symlink(&outside, fixture.root.join("linked-dir")).unwrap();
    symlink(outside.join("secret.md"), fixture.root.join("link.md")).unwrap();
    fs::hard_link(outside.join("secret.md"), fixture.root.join("hard.md")).unwrap();
    fs::create_dir(fixture.root.join("directory.md")).unwrap();
    create_fifo(&fixture.root.join("pipe.md"));
    let _socket = UnixListener::bind(fixture.root.join("socket.md")).unwrap();
    let reader = fixture.reader();
    for name in [
        ".workbench/vault.json",
        "linked-dir/secret.md",
        "link.md",
        "hard.md",
        "directory.md",
        "pipe.md",
        "socket.md",
    ] {
        assert!(
            matches!(
                reader.read_file(&path(name), 64),
                Err(BrokerError::UnsafeNode { .. })
            ),
            "{name}"
        );
    }
    assert!(matches!(
        reader.read_file(&path("missing.md"), 64),
        Err(BrokerError::NotFound(_))
    ));
    assert_eq!(
        fs::read(outside.join("secret.md")).unwrap(),
        b"outside secret"
    );
}

#[test]
fn noncanonical_paths_and_case_aliases_are_not_silently_selected() {
    let fixture = Fixture::new();
    fs::write(fixture.root.join("Mixed.md"), b"original").unwrap();
    let reader = fixture.reader();
    // 两类卷上都只差大小写：不能把 mixed.md 静默解析成磁盘上的 Mixed.md。
    assert!(matches!(
        reader.read_file(&path("mixed.md"), 64),
        Err(BrokerError::UnsafeNode {
            reason: "case-colliding filesystem name",
            ..
        })
    ));
    for (name, expected) in [
        ("../escape.md", PathSyntaxError::NotRelative),
        ("/absolute.md", PathSyntaxError::NotRelative),
        ("a/%2fsecret.md", PathSyntaxError::ForbiddenEncoding),
        ("cafe\u{301}.md", PathSyntaxError::ForbiddenEncoding),
    ] {
        assert_eq!(VaultPath::parse(name), Err(expected), "{name}");
    }
}
