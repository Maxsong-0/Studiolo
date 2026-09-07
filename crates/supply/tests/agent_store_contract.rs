//! agent 私有库契约：类型 token 只存摘要、吊销即刻生效、实例绑定类型、读取索引四维查询与重建。

use std::{fs, os::unix::fs::PermissionsExt};

use studiolo_supply::{AgentStore, ReadQuery, ReadRecord, SupplyError};
use tempfile::TempDir;

fn private_dir() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let dir = temp.path().join("agent-data");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    (temp, dir)
}

fn read(ms: i64, agent_type: &str, instance: &str, path: &str, offset: u64) -> ReadRecord {
    ReadRecord {
        recorded_at_ms: ms,
        recorded_at: format!("2026-09-07T00:00:{:02}Z", ms / 1000),
        agent_type: agent_type.into(),
        instance_id: instance.into(),
        path: path.into(),
        revision: "ab".repeat(32),
        tool: "read_note".into(),
        ledger_offset: offset,
    }
}

#[test]
fn type_tokens_are_issued_once_authenticated_by_digest_and_revoked_immediately() {
    let (_temp, dir) = private_dir();
    assert!(matches!(
        AgentStore::open_existing(&dir),
        Err(SupplyError::MissingStore)
    ));
    let store = AgentStore::open(&dir).unwrap();
    let issued = store.issue_type("claude", "2026-09-07T00:00:00Z").unwrap();
    assert!(issued.token.starts_with("sag1_"));
    assert_eq!(issued.token.len(), 5 + 64);
    // 库里没有明文。
    let bytes = fs::read(dir.join("agent.sqlite3")).unwrap();
    assert!(
        !bytes
            .windows(issued.token.len())
            .any(|w| w == issued.token.as_bytes())
    );
    assert_eq!(
        store.authenticate(&issued.token).unwrap().as_deref(),
        Some("claude")
    );
    for bad in [
        "",
        "sag1_",
        &issued.token[..60],
        "Bearer x",
        &format!("{}0", issued.token),
    ] {
        assert_eq!(store.authenticate(bad).unwrap(), None, "{bad:?}");
    }
    let mut flipped = issued.token.clone();
    flipped.replace_range(5..6, if &issued.token[5..6] == "0" { "1" } else { "0" });
    assert_eq!(store.authenticate(&flipped).unwrap(), None);
    // 轮换：旧 token 失效。
    let rotated = store.issue_type("claude", "2026-09-07T00:01:00Z").unwrap();
    assert_ne!(rotated.token, issued.token);
    assert_eq!(store.authenticate(&issued.token).unwrap(), None);
    assert!(store.type_is_active("claude").unwrap());
    // 吊销即刻生效；重新签发即恢复。
    assert!(store.revoke_type("claude", "2026-09-07T00:02:00Z").unwrap());
    assert_eq!(store.authenticate(&rotated.token).unwrap(), None);
    assert!(!store.type_is_active("claude").unwrap());
    assert!(!store.revoke_type("claude", "2026-09-07T00:02:01Z").unwrap());
    assert!(!store.revoke_type("nobody", "2026-09-07T00:02:01Z").unwrap());
    let again = store.issue_type("claude", "2026-09-07T00:03:00Z").unwrap();
    assert_eq!(
        store.authenticate(&again.token).unwrap().as_deref(),
        Some("claude")
    );
    for invalid in ["", "Claude", "a b", "-x", &"a".repeat(33)] {
        assert!(matches!(
            store.issue_type(invalid, "2026-09-07T00:00:00Z"),
            Err(SupplyError::InvalidTypeName)
        ));
    }
    assert_eq!(store.types().unwrap().len(), 1);
    // 重开只读已有库。
    drop(store);
    let reopened = AgentStore::open_existing(&dir).unwrap();
    assert_eq!(
        reopened.authenticate(&again.token).unwrap().as_deref(),
        Some("claude")
    );
}

#[test]
fn instances_bind_to_their_type_and_read_index_supports_four_dimensions_and_rebuild() {
    let (_temp, dir) = private_dir();
    let store = AgentStore::open(&dir).unwrap();
    store.issue_type("claude", "2026-09-07T00:00:00Z").unwrap();
    store.issue_type("codex", "2026-09-07T00:00:00Z").unwrap();
    let c1 = store
        .register_instance("claude", "session-1", "2026-09-07T00:00:01Z")
        .unwrap();
    let c2 = store
        .register_instance("claude", "session-2", "2026-09-07T00:00:02Z")
        .unwrap();
    let x1 = store
        .register_instance("codex", "session-3", "2026-09-07T00:00:03Z")
        .unwrap();
    assert_ne!(c1.id, c2.id);
    assert_eq!(c1.agent_type, "claude");
    assert!(
        store
            .register_instance("grok", "session-4", "2026-09-07T00:00:04Z")
            .is_err(),
        "unknown type cannot register an instance"
    );
    store
        .touch_instance(&c1.id, "2026-09-07T00:00:05Z")
        .unwrap();
    // 会话 id 不落明文。
    let bytes = fs::read(dir.join("agent.sqlite3")).unwrap();
    assert!(!bytes.windows(9).any(|w| w == b"session-1"));

    let records = [
        read(
            1_000,
            "claude",
            &c1.id,
            "项目/Studiolo/00-总览/INDEX.md",
            10,
        ),
        read(
            2_000,
            "claude",
            &c2.id,
            "项目/Studiolo/04-协调/TASKS.md",
            11,
        ),
        read(3_000, "codex", &x1.id, "项目/Studiolo/00-总览/INDEX.md", 12),
        read(4_000, "claude", &c1.id, "_规则/通用工作规则.md", 13),
    ];
    for record in &records {
        store.record_read(record).unwrap();
    }
    store.record_read(&records[0]).unwrap(); // 同一账本偏移只记一次
    let all = store
        .query_reads(&ReadQuery {
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(all, records);
    let by_type = store
        .query_reads(&ReadQuery {
            agent_type: Some("codex".into()),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(by_type, [records[2].clone()]);
    let by_instance = store
        .query_reads(&ReadQuery {
            instance_id: Some(c1.id.clone()),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(by_instance, [records[0].clone(), records[3].clone()]);
    let by_time = store
        .query_reads(&ReadQuery {
            since_ms: Some(2_000),
            until_ms: Some(3_000),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(by_time, [records[1].clone(), records[2].clone()]);
    let by_path = store
        .query_reads(&ReadQuery {
            path_prefix: Some("项目/Studiolo/00-总览/".into()),
            limit: 100,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(by_path, [records[0].clone(), records[2].clone()]);
    let combined = store
        .query_reads(&ReadQuery {
            agent_type: Some("claude".into()),
            path_prefix: Some("项目/".into()),
            since_ms: Some(1_500),
            limit: 1,
            ..ReadQuery::default()
        })
        .unwrap();
    assert_eq!(combined, [records[1].clone()]);

    let rebuilt = store.rebuild_reads(records[..2].iter().cloned()).unwrap();
    assert_eq!(rebuilt, 2);
    assert_eq!(
        store
            .query_reads(&ReadQuery {
                limit: 100,
                ..ReadQuery::default()
            })
            .unwrap(),
        records[..2]
    );
}

#[test]
fn private_storage_rules_fail_closed() {
    let (_temp, dir) = private_dir();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(matches!(
        AgentStore::open(&dir),
        Err(SupplyError::UnsafeStorage)
    ));
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let store = AgentStore::open(&dir).unwrap();
    drop(store);
    fs::set_permissions(dir.join("agent.sqlite3"), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        AgentStore::open_existing(&dir),
        Err(SupplyError::UnsafeStorage)
    ));
    assert!(AgentStore::open(std::path::Path::new("relative/dir")).is_err());
}

#[test]
fn outages_are_recorded_once_with_bounded_denial_counts() {
    let (_temp, dir) = private_dir();
    let store = AgentStore::open(&dir).unwrap();
    store
        .record_outage_started("out-1", "2026-09-07T00:00:00Z", "audit_unavailable")
        .unwrap();
    store
        .record_outage_started("out-1", "2026-09-07T00:00:01Z", "audit_unavailable")
        .unwrap();
    for _ in 0..3 {
        store.record_outage_denial("out-1").unwrap();
    }
    store
        .record_outage_ended("out-1", "2026-09-07T00:05:00Z")
        .unwrap();
    let outages = store.outages().unwrap();
    assert_eq!(outages.len(), 1);
    assert_eq!(outages[0].0, "out-1");
    assert_eq!(outages[0].1, "2026-09-07T00:00:00Z");
    assert_eq!(outages[0].2.as_deref(), Some("2026-09-07T00:05:00Z"));
    assert_eq!(outages[0].3, 3);
}
