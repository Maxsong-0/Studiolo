use super::*;
use crate::{ReadEntryKind, ReadOnlyVault, Revision};

#[test]
fn capture_preserves_true_revisions_and_complete_visible_inventory() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    for directory in ["empty", "assets", ".private"] {
        fs::create_dir(fixture.root.join(directory)).unwrap();
    }
    let bytes = b"\xef\xbb\xbf---\r\ninvalid: [\r\n---\r\n\xff";
    fs::write(fixture.root.join("note.md"), bytes).unwrap();
    fs::write(fixture.root.join("assets/blob.bin"), [0, 255, 1]).unwrap();
    fs::write(fixture.root.join(".private/secret.md"), b"fixture").unwrap();
    let before = inventory(fixture.temp.path());
    let captured = capture(&mut broker);
    assert_eq!(captured.stamp.committed_through(), 0);
    assert_eq!(captured.notes.len(), 1);
    let note = &captured.notes[0];
    assert_eq!(note.vault_uuid, broker.vault_uuid());
    assert_eq!(note.path, path("note.md"));
    assert_eq!(note.content, bytes);
    assert_eq!(
        note.revision,
        Revision::compute(broker.vault_uuid(), &note.path, bytes)
    );
    assert_eq!(captured.entries.len(), 4);
    for (name, kind, size) in [
        ("assets", ReadEntryKind::Directory, None),
        ("assets/blob.bin", ReadEntryKind::Attachment, Some(3)),
        ("empty", ReadEntryKind::Directory, None),
        ("note.md", ReadEntryKind::Markdown, Some(bytes.len() as u64)),
    ] {
        let entry = captured
            .entries
            .iter()
            .find(|entry| entry.path == path(name))
            .unwrap();
        assert_eq!((entry.kind, entry.size_bytes), (kind, size));
    }
    let replica = ReadOnlyVault::open(&fixture.root)
        .unwrap()
        .scan(ScanLimits::default())
        .unwrap();
    assert_eq!(replica.entries, captured.entries);
    assert_eq!(replica.notes[0].content, bytes);
    assert_ne!(replica.notes[0].revision, note.revision);
    broker.revalidate_capture(&captured.stamp).unwrap();
    assert_eq!(inventory(fixture.temp.path()), before);
}

#[test]
fn diagnostics_and_all_scan_limits_refuse_partial_capture() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    fs::write(fixture.root.join("note.md"), b"four").unwrap();
    assert!(matches!(
        broker.capture(ScanLimits {
            max_file_bytes: 3,
            ..Default::default()
        }),
        Err(BrokerError::IncompleteCapture)
    ));
    fs::create_dir(fixture.root.join("empty")).unwrap();
    // 总字节、枚举项与深度超限都使整次捕获失败，各自带稳定的操作标签。
    for (limits, expected) in [
        (
            ScanLimits {
                max_total_bytes: 3,
                ..Default::default()
            },
            "scan total bytes limit exceeded",
        ),
        (
            ScanLimits {
                max_entries: 1,
                ..Default::default()
            },
            "scan entry limit exceeded",
        ),
        (
            ScanLimits {
                max_depth: 0,
                ..Default::default()
            },
            "scan depth limit exceeded",
        ),
    ] {
        let result = broker.capture(limits).map(|_| ());
        assert!(
            matches!(&result, Err(BrokerError::Io { operation, .. }) if *operation == expected),
            "{expected}: {result:?}"
        );
    }
    let captured = broker
        .capture(ScanLimits {
            max_file_bytes: 4,
            max_total_bytes: 4,
            max_entries: 3,
            max_depth: 1,
        })
        .unwrap();
    assert_eq!(captured.notes.len(), 1);
    std::os::unix::fs::symlink("note.md", fixture.root.join("linked.md")).unwrap();
    assert!(matches!(
        broker.capture(ScanLimits::default()),
        Err(BrokerError::IncompleteCapture)
    ));
    assert!(!broker.write_status().unwrap().unresolved);
}

#[test]
fn single_page_read_applies_the_requested_budget_and_real_namespace() {
    let fixture = Fixture::new();
    let broker = fixture.initialize();
    let bytes = vec![b'a'; 1024 * 1024];
    fs::write(fixture.root.join("note.md"), &bytes).unwrap();
    let note = broker.read_file(&path("note.md"), 1024 * 1024).unwrap();
    assert_eq!(note.content, bytes);
    assert_eq!(
        note.revision,
        Revision::compute(broker.vault_uuid(), &note.path, &bytes)
    );
    assert_eq!(note.vault_uuid, broker.vault_uuid());
    assert!(matches!(
        broker.read_file(&path("note.md"), 1024 * 1024 - 1),
        Err(BrokerError::ReadLimitExceeded {
            max_bytes: 1_048_575,
            ..
        })
    ));
    fs::write(fixture.root.join("empty.md"), b"").unwrap();
    assert!(
        broker
            .read_file(&path("empty.md"), 0)
            .unwrap()
            .content
            .is_empty()
    );
    assert!(matches!(
        broker.read_file(&path(".workbench/vault.json"), 4096),
        Err(BrokerError::UnsafeNode { .. })
    ));
    fs::hard_link(fixture.root.join("note.md"), fixture.root.join("linked.md")).unwrap();
    assert!(matches!(
        broker.read_file(&path("linked.md"), 1024 * 1024),
        Err(BrokerError::UnsafeNode { .. })
    ));
}

#[test]
fn external_edit_is_not_misrepresented_as_an_observed_broker_commit() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    fs::write(fixture.root.join("note.md"), b"before").unwrap();
    let captured = capture(&mut broker);
    fs::write(fixture.root.join("note.md"), b"external").unwrap();
    // stamp 只核本写者水位与绑定；带外改写由上层 watcher/维护约束处理。
    broker.revalidate_capture(&captured.stamp).unwrap();
    assert_eq!(captured.notes[0].content, b"before");
    assert_eq!(
        broker.read_file(&path("note.md"), 64).unwrap().content,
        b"external"
    );
    let rescanned = capture(&mut broker);
    assert_eq!(rescanned.notes[0].content, b"external");
    assert_ne!(captured.notes[0].revision, rescanned.notes[0].revision);
}

#[test]
fn canonical_conversion_uses_captured_bytes_without_a_second_file_read() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    let target = fixture.root.join("note.md");
    fs::write(&target, b"captured").unwrap();
    let _hook = read_hook(move || fs::write(target, b"external").unwrap());
    let captured = capture(&mut broker);
    assert_eq!(captured.notes[0].content, b"captured");
    assert_eq!(
        captured.notes[0].revision,
        Revision::compute(broker.vault_uuid(), &path("note.md"), b"captured")
    );
    assert_eq!(
        broker.read_file(&path("note.md"), 64).unwrap().content,
        b"external"
    );
}
