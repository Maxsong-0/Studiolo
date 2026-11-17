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

#[test]
fn paired_checkpoint_uses_only_fixed_agent_audit_and_original_operation_gate() {
    let dir = directory();
    let human_root = directory();
    let audit_root = dir.path().join("audit");
    fs::create_dir(&audit_root).unwrap();
    fs::set_permissions(&audit_root, fs::Permissions::from_mode(0o700)).unwrap();
    let store = AgentStore::open(dir.path()).unwrap();
    store.issue_type("codex", NOW).unwrap();
    let instance = store.register_instance("codex", "session", NOW).unwrap();
    let agent = SqliteAuditLedger::open(&audit_root).unwrap();
    let event = audit::content_read(
        Actor::instance("codex", &instance.id),
        "notes/test.md",
        "aabb",
        "read_note",
    )
    .unwrap();
    let record = agent.append(event).unwrap();
    store
        .record_read(&ReadRecord {
            recorded_at_ms: record.recorded_at().as_millisecond(),
            recorded_at: record.recorded_at().to_string(),
            agent_type: "codex".into(),
            instance_id: instance.id,
            path: "notes/test.md".into(),
            revision: "aabb".into(),
            tool: "read_note".into(),
            ledger_offset: record.offset(),
        })
        .unwrap();
    let human = SqliteAuditLedger::open(human_root.path()).unwrap();
    human
        .append(
            audit::content_read(
                Actor::instance("codex", "different"),
                "notes/human.md",
                "ccdd",
                "read_note",
            )
            .unwrap(),
        )
        .unwrap();
    let view = AgentIdentityView::open_existing(dir.path()).unwrap();
    let pair = view.checkpoint_assembly().unwrap();
    let active = agent.enter_operation().unwrap();
    assert!(
        pair.quiesce(Instant::now() + Duration::from_millis(30))
            .is_err()
    );
    drop(active);
    let quiet = pair.quiesce(deadline()).unwrap();
    // 人侧许可不被这个独立根的排空错误地关闭。
    let human_operation = human.enter_operation().unwrap();
    drop(human_operation);
    let mut frozen = pair.freeze_store(deadline()).unwrap();
    let wrong = human
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert!(frozen.verify_audit(&wrong).is_err());
    let original = pair.freeze_audit(deadline()).unwrap();
    assert_eq!(frozen.verify_audit(&original).unwrap().checked(), 1);
    frozen.export(directory().path()).unwrap();
    drop(original);
    drop(frozen);
    drop(quiet);
    assert!(agent.enter_operation().is_ok());
}
#[test]
fn paired_checkpoint_does_not_initialize_missing_audit_or_rebind_replaced_identity() {
    let dir = directory();
    let store = AgentStore::open(dir.path()).unwrap();
    store.issue_type("codex", NOW).unwrap();
    let view = AgentIdentityView::open_existing(dir.path()).unwrap();
    assert!(view.checkpoint_assembly().is_err());
    assert!(!dir.path().join("audit").exists());
    let audit_root = dir.path().join("audit");
    fs::create_dir(&audit_root).unwrap();
    fs::set_permissions(&audit_root, fs::Permissions::from_mode(0o700)).unwrap();
    let _ledger = SqliteAuditLedger::open(&audit_root).unwrap();
    let pair = view.checkpoint_assembly().unwrap();
    let database = dir.path().join("agent.sqlite3");
    fs::rename(&database, dir.path().join("replaced.sqlite3")).unwrap();
    fs::copy(dir.path().join("replaced.sqlite3"), &database).unwrap();
    assert!(pair.freeze_store(deadline()).is_err());
    assert!(view.checkpoint_assembly().is_err());
}

#[test]
fn shared_identity_assembly_also_requires_the_same_original_audit() {
    let dir = directory();
    drop(AgentStore::open(dir.path()).unwrap());
    let audit = dir.path().join("audit");
    fs::create_dir(&audit).unwrap();
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o700)).unwrap();
    drop(SqliteAuditLedger::open(&audit).unwrap());
    let view = AgentIdentityView::open_existing(dir.path()).unwrap();
    let pair = view.checkpoint_assembly().unwrap();
    assert!(
        pair.same_origin(&view.checkpoint_assembly().unwrap())
            .unwrap()
    );
    let retained = dir.path().join("old-audit");
    fs::rename(&audit, &retained).unwrap();
    fs::create_dir(&audit).unwrap();
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o700)).unwrap();
    drop(SqliteAuditLedger::open(&audit).unwrap());
    let replacement = view.checkpoint_assembly().unwrap();
    assert!(pair.same_origin(&replacement).is_err());
    assert!(replacement.same_origin(&pair).is_err());
}

#[test]
fn restored_identities_lose_every_old_token_and_hand_back_every_instance() {
    let dir = directory();
    let store = AgentStore::open(dir.path()).unwrap();
    let codex = store.issue_type("codex", NOW).unwrap();
    store.issue_type("claude", NOW).unwrap();
    let first = store.register_instance("codex", "session-1", NOW).unwrap();
    let second = store.register_instance("claude", "session-2", NOW).unwrap();
    // 封存时同一套规则算出来的水位，就是恢复要对着核的那一份。
    let expected = {
        let view = AgentIdentityView::open_existing(dir.path()).unwrap();
        let frozen = view
            .checkpoint_source()
            .unwrap()
            .freeze(deadline())
            .unwrap();
        frozen.watermark().clone()
    };
    drop(store);
    // 水位对不上（这里拿另一个库的）核对不过；核对只读，什么都不动。
    let other = directory();
    let foreign = AgentStore::open(other.path()).unwrap();
    foreign.issue_type("codex", NOW).unwrap();
    let foreign_mark = {
        let view = AgentIdentityView::open_existing(other.path()).unwrap();
        let frozen = view
            .checkpoint_source()
            .unwrap()
            .freeze(deadline())
            .unwrap();
        frozen.watermark().clone()
    };
    assert!(studiolo_supply::verify_restored_identities(dir.path(), &foreign_mark).is_err());
    studiolo_supply::verify_restored_identities(dir.path(), &expected).unwrap();
    let untouched = AgentStore::open_existing(dir.path()).unwrap();
    assert_eq!(
        untouched.authenticate(&codex.token).unwrap(),
        Some("codex".into())
    );
    drop(untouched);

    let done = studiolo_supply::invalidate_restored_identities(dir.path(), NOW).unwrap();
    assert_eq!(done.types_revoked, 2);
    assert_eq!(done.types_total, 2);
    let mut ids = vec![first.id.clone(), second.id.clone()];
    ids.sort();
    assert_eq!(done.instances, ids);
    let reopened = AgentStore::open_existing(dir.path()).unwrap();
    assert_eq!(reopened.authenticate(&codex.token).unwrap(), None);
    assert!(
        reopened
            .types()
            .unwrap()
            .iter()
            .all(|(_, _, revoked)| *revoked)
    );
    drop(reopened);
    // 失效之后库已经不是封存那一份：再核水位不过（接手时不再核，只做幂等的失效）；再做一遍什么都不变。
    assert!(studiolo_supply::verify_restored_identities(dir.path(), &expected).is_err());
    let again = studiolo_supply::invalidate_restored_identities(dir.path(), NOW).unwrap();
    assert_eq!(again.types_revoked, 0);
    assert_eq!(again.instances, ids);
    // 不建库：目录里没有库，核对和失效都失败，也不留下新文件。
    let empty = directory();
    assert!(studiolo_supply::verify_restored_identities(empty.path(), &expected).is_err());
    assert!(studiolo_supply::invalidate_restored_identities(empty.path(), NOW).is_err());
    assert_eq!(fs::read_dir(empty.path()).unwrap().count(), 0);
}
