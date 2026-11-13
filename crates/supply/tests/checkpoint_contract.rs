//! AgentStore 两种真实格式均可封存；索引必须对应接入方自己的完整审计事件。
#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_supply::{
    AgentIdentityView, AgentStore, ReadRecord,
    audit::{self, Actor},
};
fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
const NOW: &str = "2026-11-13T20:00:00Z";
#[test]
fn both_identity_formats_preserve_tokens_lineages_and_real_read_receipts() {
    for children in [false, true] {
        let dir = directory();
        let audit_dir = directory();
        let target = directory();
        let store = AgentStore::open(dir.path()).unwrap();
        let token = store.issue_type("codex", NOW).unwrap();
        let root = store
            .register_instance("codex", "root-session", NOW)
            .unwrap();
        let instance = if children {
            store.upgrade_child_instances().unwrap();
            store
                .register_child_instance("codex", &root.id, "child-session", NOW)
                .unwrap()
        } else {
            root.clone()
        };
        let ledger = SqliteAuditLedger::open(audit_dir.path()).unwrap();
        let event = audit::content_read(
            Actor::instance("codex", &instance.id),
            "notes/test.md",
            "aabb",
            "read_note",
        )
        .unwrap();
        let record = ledger.append(event).unwrap();
        store
            .record_read(&ReadRecord {
                recorded_at_ms: record.recorded_at().as_millisecond(),
                recorded_at: record.recorded_at().to_string(),
                agent_type: "codex".into(),
                instance_id: instance.id.clone(),
                path: "notes/test.md".into(),
                revision: "aabb".into(),
                tool: "read_note".into(),
                ledger_offset: record.offset(),
            })
            .unwrap();
        let view = AgentIdentityView::open_existing(dir.path()).unwrap();
        let source = view.checkpoint_source().unwrap();
        let mut frozen = source.freeze(deadline()).unwrap();
        assert!(frozen.export(target.path()).is_err());
        let audit = ledger
            .checkpoint_source()
            .unwrap()
            .freeze(deadline())
            .unwrap();
        assert_eq!(frozen.verify_audit(&audit).unwrap().checked(), 1);
        frozen.export(target.path()).unwrap();
        let copy = AgentStore::open_existing(target.path()).unwrap();
        assert_eq!(
            copy.authenticate(&token.token).unwrap(),
            Some("codex".into())
        );
        assert_eq!(
            copy.instance_lineage(&instance.id).unwrap().len(),
            if children { 2 } else { 1 }
        );
        let raw = rusqlite::Connection::open(dir.path().join("agent.sqlite3")).unwrap();
        raw.busy_timeout(Duration::ZERO).unwrap();
        assert!(
            raw.execute("UPDATE reads SET path='notes/wrong.md'", [])
                .is_err()
        );
        drop(frozen);
        raw.execute("UPDATE reads SET path='notes/wrong.md'", [])
            .unwrap();
        let mut frozen = source.freeze(deadline()).unwrap();
        assert!(frozen.verify_audit(&audit).is_err());
        assert!(frozen.export(directory().path()).is_err());
    }
}
#[test]
fn an_open_outage_or_damaged_identity_blocks_the_checkpoint() {
    let dir = directory();
    let store = AgentStore::open(dir.path()).unwrap();
    store.issue_type("codex", NOW).unwrap();
    let view = AgentIdentityView::open_existing(dir.path()).unwrap();
    let source = view.checkpoint_source().unwrap();
    store
        .record_outage_started("outage-1", NOW, "audit_unavailable")
        .unwrap();
    store.record_outage_denial("outage-1").unwrap();
    assert!(source.freeze(deadline()).is_err());
    store.record_outage_ended("outage-1", NOW).unwrap();
    drop(source.freeze(deadline()).unwrap());
    let raw = rusqlite::Connection::open(dir.path().join("agent.sqlite3")).unwrap();
    raw.execute("UPDATE agent_type SET token_sha256=?1", ["X".repeat(64)])
        .unwrap();
    assert!(source.freeze(deadline()).is_err());
}
