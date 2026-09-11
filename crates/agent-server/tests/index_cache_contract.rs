//! 持久派生索引只复用计算；真实来源验证、身份与审计仍决定供给。
#[allow(dead_code)]
#[path = "http_text/support.rs"]
mod http_support;
mod index_cache_support;
#[path = "../../supply/tests/replica_support/mod.rs"]
mod replica_support;

use index_cache_support::{Fixture, replica_options};
use replica_support::{ReplicaFixture, SECOND, private_dir, write};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, sync::Arc};
use studiolo_agent_server::{
    index::{ModelSource, Persistence},
    serve,
};
use studiolo_core::VaultPath;
use studiolo_supply::WorkbenchReadModel;

#[test]
fn cold_memory_restart_and_incremental_models_match_current_source() {
    let fixture = Fixture::new();
    fixture.note("a.md", "# A\n\n[[b]]\n");
    fixture.note("b.md", "# B\n\n旧正文\n");
    fixture.note("c.md", "# C\n");
    let state = serve::prepare(&fixture.options).unwrap();
    let first = state.current();
    assert_eq!(first.index_status.source, ModelSource::Rebuilt);
    assert_eq!(
        (first.index_status.parsed, first.index_status.reused),
        (3, 0)
    );
    assert_eq!(first.index_status.persistence, Persistence::Ready);
    state.rescan().unwrap();
    let warm = state.current();
    assert!(Arc::ptr_eq(&first.model, &warm.model));
    assert_eq!(warm.index_status.source, ModelSource::Memory);
    assert_eq!((warm.index_status.parsed, warm.index_status.reused), (0, 3));
    drop(state);

    let state = serve::prepare(&fixture.options).unwrap();
    let restored = state.current();
    assert_eq!(restored.index_status.source, ModelSource::Disk);
    assert_eq!(
        (restored.index_status.parsed, restored.index_status.reused),
        (0, 3)
    );
    assert_eq!(restored.model.snapshot(), first.model.snapshot());
    let a = VaultPath::parse("project/a.md").unwrap();
    assert_eq!(restored.model.navigation(&a), first.model.navigation(&a));

    fixture.note("b.md", "# B 新标题\n\n本次新正文\n");
    state.rescan().unwrap();
    let updated = state.current();
    assert!(!Arc::ptr_eq(&restored.model, &updated.model));
    assert_eq!(updated.index_status.source, ModelSource::Incremental);
    assert_eq!(
        (updated.index_status.parsed, updated.index_status.reused),
        (1, 2)
    );
    let fresh = WorkbenchReadModel::build(&updated.snapshot.documents()).unwrap();
    assert_eq!(updated.model.snapshot(), fresh.snapshot());
    assert_eq!(updated.model.navigation(&a), fresh.navigation(&a));
    let b = VaultPath::parse("project/b.md").unwrap();
    assert_eq!(updated.model.document(&b), fresh.document(&b));
    assert!(
        updated
            .model
            .document(&b)
            .unwrap()
            .content
            .contains("本次新正文")
    );
    assert!(
        !updated
            .model
            .document(&b)
            .unwrap()
            .content
            .contains("旧正文")
    );
}

#[test]
fn one_changed_note_in_128_reuses_the_other_127_after_restart() {
    let fixture = Fixture::new();
    for number in 0..128 {
        fixture.note(
            &format!("note-{number}.md"),
            &format!("# 文档 {number}\n\n正文。\n"),
        );
    }
    let cold = serve::prepare(&fixture.options).unwrap();
    assert_eq!(cold.current().index_status.parsed, 128);
    drop(cold);
    let restarted = serve::prepare(&fixture.options).unwrap();
    assert_eq!(
        (
            restarted.current().index_status.parsed,
            restarted.current().index_status.reused
        ),
        (0, 128)
    );
    fixture.note("note-64.md", "# 文档 64\n\n独立变更。\n");
    restarted.rescan().unwrap();
    let result = restarted.current();
    assert_eq!(
        (result.index_status.parsed, result.index_status.reused),
        (1, 127)
    );
    println!("128 篇：冷启动解析 128，重启解析 0，单篇变更解析 1、复用 127");
}

#[test]
fn corrupt_database_falls_back_to_fresh_documents_without_overwriting_it() {
    let fixture = Fixture::new();
    fixture.note("note.md", "# 初始\n");
    drop(serve::prepare(&fixture.options).unwrap());
    let database = fixture.cache.join("index.sqlite3");
    let broken = b"damaged sqlite cache";
    write(&database, broken);
    fixture.note("note.md", "# 新来源\n\n可靠正文\n");
    let state = serve::prepare(&fixture.options).unwrap();
    let current = state.current();
    assert_eq!(current.index_status.source, ModelSource::Rebuilt);
    assert_eq!(current.index_status.persistence, Persistence::Unavailable);
    assert_eq!(current.index_status.parsed, 1);
    assert!(current.snapshot.notes[0].content.contains("可靠正文"));
    assert_eq!(fs::read(database).unwrap(), broken);
}

#[test]
fn overlapping_or_nonprivate_cache_roots_refuse_before_database_creation() {
    let fixture = Fixture::new();
    fixture.note("note.md", "# 来源\n");
    let nested_source = fixture.vault.join("project/cache");
    let nested_data = fixture.options.data.join("cache");
    for root in [&nested_source, &nested_data] {
        private_dir(root);
    }
    for root in [
        fixture.vault.as_path(),
        nested_source.as_path(),
        fixture.options.data.as_path(),
        nested_data.as_path(),
        fixture.temp.path(),
    ] {
        let mut options = fixture.options.clone();
        options.index_cache = Some(root.to_owned());
        assert!(serve::prepare(&options).is_err(), "未拒绝重叠根 {root:?}");
        assert!(!root.join("index.sqlite3").exists());
    }
    fs::set_permissions(&fixture.cache, fs::Permissions::from_mode(0o750)).unwrap();
    assert!(serve::prepare(&fixture.options).is_err());
    assert!(!fixture.cache.join("index.sqlite3").exists());
}

#[tokio::test]
async fn cached_replica_rechecks_receipts_sync_status_and_source_tampering() {
    let replica = ReplicaFixture::new();
    let options = replica_options(&replica);
    let state = serve::prepare(&options).unwrap();
    let before = state.current();
    replica.publish(SECOND, "oldmarker", replica.observed + 1000);
    let confirmed = state.verified().await.unwrap();
    assert!(Arc::ptr_eq(&before.model, &confirmed.model));
    assert_eq!(confirmed.index_status.source, ModelSource::Memory);
    assert_ne!(confirmed.freshness, before.freshness);
    assert_eq!(confirmed.index_status.parsed, 0);

    let mut receipt = replica.state();
    receipt["status"] = json!("failed");
    receipt["last_error"] = json!("transfer_failed");
    receipt["last_attempt_at_ms"] = json!(jiff::Timestamp::now().as_millisecond());
    replica.write_state(&receipt);
    let failed_sync = state.verified().await.unwrap();
    assert!(Arc::ptr_eq(&confirmed.model, &failed_sync.model));
    assert_ne!(failed_sync.freshness, confirmed.freshness);
    write(
        &replica.release(SECOND).join("vault/project/note.md"),
        b"# injected body",
    );
    assert!(state.verified().await.is_err());
    assert_eq!(state.current().number, failed_sync.number);
    assert!(
        !state
            .current()
            .snapshot
            .notes
            .iter()
            .any(|note| note.content.contains("injected body"))
    );
}

#[tokio::test]
async fn cached_http_reads_still_require_current_auth_and_writable_audit() {
    let replica = ReplicaFixture::new();
    let server = http_support::spawn(replica_options(&replica)).await;
    let (session, _, _) = server.handshake().await;
    assert_eq!(
        server.state.current().index_status.source,
        ModelSource::Memory
    );
    let route = "/read/note?path=project%2Fnote.md";
    let valid = server.get(route, Some(&session)).send().await.unwrap();
    assert_eq!(valid.status(), 200);
    assert!(valid.text().await.unwrap().contains("oldmarker"));
    let invalid = server
        .request(reqwest::Method::GET, route, Some("invalid"), Some(&session))
        .send()
        .await
        .unwrap();
    let body = http_support::denied(invalid, 401).await;
    assert!(!body.contains("oldmarker"));

    let audit = server.data.join("audit");
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o750)).unwrap();
    let body =
        http_support::denied(server.get(route, Some(&session)).send().await.unwrap(), 503).await;
    assert!(body.contains("审计不可用"));
    assert!(!body.contains("oldmarker"));
    fs::set_permissions(&audit, fs::Permissions::from_mode(0o700)).unwrap();
    server
        .store()
        .revoke_type("claude", "2026-09-10T01:00:00Z")
        .unwrap();
    let body =
        http_support::denied(server.get(route, Some(&session)).send().await.unwrap(), 401).await;
    assert!(!body.contains("oldmarker"));
}
