//! 真实跨连接、倒序分页和段完整性；不靠源码证明只读。
#![cfg(unix)]
mod support;
use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditReadView, SanitizedPayload,
    SqliteAuditLedger,
};
fn event(n: usize) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(format!("read-test:{n}")).unwrap(),
        AuditPhase::Outcome,
        "test",
        SanitizedPayload::try_from(json!({"n":n})).unwrap(),
    )
}
#[test]
fn readonly_pages_are_stable_across_appends_and_verify_every_selected_segment() {
    let dir = support::private_dir();
    let path = dir.path().canonicalize().unwrap();
    assert!(matches!(
        AuditReadView::open_existing(&path),
        Err(AuditError::MissingStore)
    ));
    let writer = SqliteAuditLedger::open(&path).unwrap();
    let view = AuditReadView::open_existing(&path).unwrap();
    assert_eq!(
        view.scan_page(None, |r| Ok(Some(r.offset())))
            .unwrap()
            .scanned,
        0
    );
    for n in 1..=600 {
        writer.append(event(n)).unwrap();
    }
    let newest = view.scan_page(None, |r| Ok(Some(r.offset()))).unwrap();
    assert_eq!(newest.scanned, 256);
    assert_eq!(newest.records, (345..=600).rev().collect::<Vec<_>>());
    assert_eq!(newest.next_before, Some(345));
    writer.append(event(601)).unwrap();
    let middle = view
        .scan_page(newest.next_before, |r| Ok(Some(r.offset())))
        .unwrap();
    assert_eq!(middle.head_offset, 601);
    assert_eq!(middle.records, (89..=344).rev().collect::<Vec<_>>());
    let end = view
        .scan_page(middle.next_before, |_| Ok(None::<u64>))
        .unwrap();
    assert_eq!(end.scanned, 88);
    assert!(end.next_before.is_none());
    // 篡改已封存的旧段；最新页不假称检查旧段，翻到旧段必须失败。
    let raw = rusqlite::Connection::open(path.join("audit.sqlite3")).unwrap();
    raw.execute(
        "UPDATE audit_events SET record_json='{}' WHERE offset=5",
        [],
    )
    .unwrap();
    assert!(view.scan_page(None, |r| Ok(Some(r.offset()))).is_ok());
    assert!(matches!(
        view.scan_page(Some(89), |r| Ok(Some(r.offset()))),
        Err(AuditError::CorruptStore)
    ));
}

#[test]
fn replaced_directory_and_unknown_schema_are_rejected() {
    let dir = support::private_dir();
    let path = dir.path().canonicalize().unwrap();
    let writer = SqliteAuditLedger::open(&path).unwrap();
    let view = AuditReadView::open_existing(&path).unwrap();
    let raw = rusqlite::Connection::open(path.join("audit.sqlite3")).unwrap();
    raw.execute("UPDATE audit_meta SET schema_version=99 WHERE id=1", [])
        .unwrap();
    assert!(AuditReadView::open_existing(&path).is_err());
    assert!(view.scan_page(None, |_| Ok(None::<u8>)).is_err());
    drop(raw);
    drop(writer);
    let container = support::private_dir();
    let parent = container.path().canonicalize().unwrap();
    let current = parent.join("current");
    std::fs::create_dir(&current).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _writer = SqliteAuditLedger::open(&current).unwrap();
    let view = AuditReadView::open_existing(&current).unwrap();
    std::fs::rename(&current, parent.join("old")).unwrap();
    std::fs::create_dir(&current).unwrap();
    std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(
        view.scan_page(None, |_| Ok(None::<u8>)),
        Err(AuditError::UnsafeStorage)
    ));
}
