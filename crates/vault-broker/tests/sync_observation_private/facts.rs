use super::*;
use sha2::{Digest, Sha256};

#[test]
fn complete_local_files_missing_and_exact_file_rules_share_one_capture() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/中文.md", "真正正文\n".as_bytes());
    f.write("attachment.bin", &[0xff, 0x00, 0x80]);
    fs::create_dir(f.root.join("notes/empty")).unwrap();
    let mut selection = selection();
    selection.files = vec![
        SyncReadRule {
            path: path("attachment.bin"),
            max_bytes: 3,
        },
        SyncReadRule {
            path: path("new-parent/never.txt"),
            max_bytes: 100,
        },
    ];
    let result = broker
        .capture_sync_observation(&selection, &[path("notes/deep/missing.md")])
        .unwrap();
    assert_eq!(result.entries().len(), 4);
    assert_eq!(
        result.entries()[&path("notes/deep/missing.md")],
        CanonicalSyncPresence::Missing
    );
    assert_eq!(
        result.entries()[&path("new-parent/never.txt")],
        CanonicalSyncPresence::Missing
    );
    let text = present(&result, "notes/中文.md");
    assert!(text.utf8_valid);
    assert_eq!(
        text.revision,
        crate::Revision::compute(
            result.vault_uuid(),
            &path("notes/中文.md"),
            "真正正文\n".as_bytes()
        )
    );
    assert_eq!(
        text.identity.sha256,
        crate::hex(&Sha256::digest("真正正文\n".as_bytes()))
    );
    assert!(!present(&result, "attachment.bin").utf8_valid);
    assert_eq!(present(&result, "attachment.bin").identity.bytes, 3);
    broker.revalidate_sync_observation(&result).unwrap();
}

#[test]
fn empty_anchor_and_explicit_missing_are_not_omitted() {
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    assert!(capture(&mut broker).entries().is_empty());
    let result = broker
        .capture_sync_observation(&selection(), &[path("notes/a.md")])
        .unwrap();
    assert_eq!(
        result.entries()[&path("notes/a.md")],
        CanonicalSyncPresence::Missing
    );
}

#[test]
fn utf8_fact_checks_cross_chunk_sequences_and_eof_without_format_classification() {
    let f = Fixture::new();
    let mut broker = f.broker();
    let mut complete = vec![b'a'; 65535];
    complete.extend_from_slice("😀中文".as_bytes());
    f.write("notes/valid", &complete);
    f.write("notes/truncated", &[0xf0, 0x9f, 0x98]);
    let mut invalid = vec![b'a'; 65535];
    invalid.extend_from_slice(&[0xf0, 0x80, 0x80, 0x80]);
    f.write("notes/overlong", &invalid);
    f.write("notes/nul", &[0]);
    f.write("notes/empty", &[]);
    f.write("notes/pdf-magic-only", b"%PDF-1.7\n");
    let result = capture(&mut broker);
    assert!(present(&result, "notes/valid").utf8_valid);
    assert!(!present(&result, "notes/truncated").utf8_valid);
    assert!(!present(&result, "notes/overlong").utf8_valid);
    assert!(present(&result, "notes/nul").utf8_valid);
    assert!(present(&result, "notes/empty").utf8_valid);
    // UTF8=true 只是字节事实：并不把截断 PDF 或 NUL 文本当格式/Markdown 策略有效。
    assert!(present(&result, "notes/pdf-magic-only").utf8_valid);
}

#[test]
fn excluded_subtrees_are_not_traversed_and_excluded_bodies_are_never_opened() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/good", b"ok");
    f.write("notes/TASKS.md", b"human task untouched");
    f.write("notes/generated.md", b"sentinels removed");
    f.write("notes/private/inside", &[0xff]);
    f.write("notes/ops/tasks/a", b"runtime");
    f.write("notes/raw/a", b"source");
    f.write("outside.md", b"not approved");
    std::os::unix::fs::symlink(
        "/definitely-not-readable",
        f.root.join("notes/private/link"),
    )
    .unwrap();
    reset();
    let result = capture(&mut broker);
    assert_eq!(result.entries().len(), 1);
    assert_eq!(count(1), 1);
    assert_eq!(
        fs::read(f.root.join("notes/TASKS.md")).unwrap(),
        b"human task untouched"
    );
}
