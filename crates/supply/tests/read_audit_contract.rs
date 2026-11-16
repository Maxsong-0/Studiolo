//! 读取与拒绝使用真实权威账本；索引缺失不能把已交付内容隐藏。
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_supply::{
    AgentIdentityView, AgentStore,
    audit::{self, Actor},
    read_audit::{ReadAuditQuery, ReadOutcome},
};
const ID: &str = "01K00000000000000000000001";
fn query(outcome: ReadOutcome) -> ReadAuditQuery {
    ReadAuditQuery {
        outcome,
        agent_type: None,
        instance_id: None,
        since: None,
        until: None,
        path_prefix: None,
        before: None,
    }
}
#[test]
fn authoritative_read_filters_and_denials_never_invent_paths() {
    let tmp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
    let path = tmp.path();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(path.join("audit")).unwrap();
    fs::set_permissions(path.join("audit"), fs::Permissions::from_mode(0o700)).unwrap();
    let _identity_writer = AgentStore::open(path).unwrap();
    let ledger = SqliteAuditLedger::open(&path.join("audit")).unwrap();
    let read = AgentIdentityView::open_existing(path)
        .unwrap()
        .read_audit()
        .unwrap();
    let actor = Actor::instance("claude", ID);
    let first = ledger
        .append(audit::content_read(actor, "项目/A/a.md", "abc", "read_note").unwrap())
        .unwrap();
    ledger
        .append(
            audit::content_read(
                Actor::instance("codex", ID),
                "policy://rules/a",
                "v2",
                "policy_read",
            )
            .unwrap(),
        )
        .unwrap();
    ledger
        .append(audit::action_denied(actor, "read_note", "capability_denied").unwrap())
        .unwrap();
    ledger
        .append(audit::action_denied(Actor::anonymous(), "mcp.request", "unauthenticated").unwrap())
        .unwrap();
    let mut q = query(ReadOutcome::Read);
    q.agent_type = Some("claude".into());
    q.instance_id = Some(ID.into());
    q.path_prefix = Some("项目/A/".into());
    q.since = Some(first.recorded_at().to_string());
    q.until = q.since.clone();
    let page = read.query(&q).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].offset, first.offset());
    q.path_prefix = Some("项目/A/%".into());
    assert!(read.query(&q).unwrap().records.is_empty());
    let denied = read.query(&query(ReadOutcome::Denied)).unwrap();
    assert_eq!(denied.records.len(), 2);
    assert!(
        denied
            .records
            .iter()
            .all(|r| r.path.is_none() && r.revision.is_none())
    );
    assert!(denied.records[0].agent_type.is_none());
    let mut bad = query(ReadOutcome::Denied);
    bad.path_prefix = Some("项目/".into());
    assert!(!bad.validate());
    bad.path_prefix = None;
    bad.before = Some(0);
    assert!(!bad.validate());
    bad.before = None;
    bad.since = Some("tomorrow".into());
    assert!(!bad.validate());
    for _ in 0..260 {
        ledger
            .append(audit::action_denied(actor, "read_note", "capability_denied").unwrap())
            .unwrap();
    }
    let mut all = query(ReadOutcome::Read);
    let empty = read.query(&all).unwrap();
    assert!(empty.records.is_empty());
    assert!(empty.next_before.is_some());
    all.before = empty.next_before;
    assert_eq!(read.query(&all).unwrap().records.len(), 2);
    let encoded = serde_json::to_value(denied).unwrap();
    assert_eq!(encoded.as_object().unwrap().len(), 6);
    assert_eq!(encoded["records"][0].as_object().unwrap().len(), 8);
}
