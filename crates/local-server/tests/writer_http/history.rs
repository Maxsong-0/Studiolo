//! 真实会话、L0与Audit闭环；恢复使用独立DTO，不能冒充普通编辑降级。
use crate::{
    editing::{body, refresh, save},
    fixture::Fixture,
};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};

#[derive(Default)]
struct SwitchAudit {
    ledger: MemoryAuditLedger,
    mode: AtomicU8,
}
impl AuditLedger for SwitchAudit {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let mode = self.mode.load(Ordering::Acquire);
        if mode == 1
            || (mode == 2
                && event.action() == "recovery.page_restore"
                && event.phase() == AuditPhase::Outcome)
        {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}
fn restore(f: &Fixture, expected: &Value, key: &str) -> Value {
    json!({"kind":"restore","vault_uuid":f.expected.vault_uuid.to_string(),"path":"a.md",
        "expected_revision":expected,"source_revision":f.expected.revision.to_hex(),"idempotency_key":key})
}
fn history_url(f: &Fixture) -> String {
    format!(
        "/api/v1/history?vault_uuid={}&path=a.md",
        f.expected.vault_uuid
    )
}

#[tokio::test]
async fn history_preserves_unknown_original_author_and_records_the_actual_archiver() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    save(
        &f,
        &issued,
        &body(&f, "# 新版\n", "history-metadata-first"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    let page = f.get(&history_url(&f), &issued.cookie, 200).await;
    assert_eq!(page["vault_uuid"], f.expected.vault_uuid.to_string());
    assert_eq!(page["path"], "a.md");
    let initial = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["revision"] == f.expected.revision.to_hex())
        .unwrap();
    assert!(initial["author"].is_null() && initial["authored_at_ms"].is_null());
    assert_eq!(initial["archived_by"]["kind"], "human");
    assert_eq!(initial["archived_by"]["subject"], "fictional-person");
    assert!(initial["archived_at_ms"].as_i64().unwrap() > 0);
    let old = f
        .get(
            &format!(
                "/api/v1/history/file?vault_uuid={}&path=a.md&revision={}",
                f.expected.vault_uuid,
                f.expected.revision.to_hex()
            ),
            &issued.cookie,
            200,
        )
        .await;
    assert_eq!(
        old["content"],
        String::from_utf8(f.expected.content.clone()).unwrap()
    );
    let mut cross_path = history_url(&f).replace("path=a.md", "path=CLAUDE.md");
    assert_eq!(
        f.get(&cross_path, &issued.cookie, 403).await["code"],
        "protected"
    );
    cross_path = history_url(&f).replace(
        &f.expected.vault_uuid.to_string(),
        &studiolo_core::VaultUuid::new().to_string(),
    );
    f.get(&cross_path, &issued.cookie, 403).await;
    f.close().await;
}

#[tokio::test]
async fn normal_edit_survives_audit_outage_and_later_flushes_exact_metadata_without_body() {
    let audit = Arc::new(SwitchAudit::default());
    let f = Fixture::with_audit(audit.clone()).await;
    let issued = f.http.issue().await;
    audit.mode.store(1, Ordering::Release);
    let private = "# 唯一私有正文不得进入审计测试字符串\n";
    save(&f, &issued, &body(&f, private, "history-outage-write"), 200).await;
    assert_eq!(f.journal()["committed_through"], 1);
    assert!(audit.query(&AuditFilter::default()).unwrap().is_empty());
    audit.mode.store(0, Ordering::Release);
    refresh(&f, &issued).await;
    let events = audit
        .query(&AuditFilter::default().with_action("vault.mutation"))
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event().phase(), AuditPhase::Intent);
    assert_eq!(events[1].event().phase(), AuditPhase::Outcome);
    assert_eq!(events[0].event().event_id(), events[1].event().event_id());
    let encoded = serde_json::to_string(&events).unwrap();
    assert!(!encoded.contains(private));
    assert!(encoded.contains("fictional-person") && encoded.contains("committed"));
    assert_eq!(
        fs::read_to_string(f.http.root.join("vault/a.md")).unwrap(),
        private
    );
    f.close().await;
}

#[tokio::test]
async fn restore_requires_actual_intent_append_even_when_audit_queries_still_work() {
    let audit = Arc::new(SwitchAudit::default());
    let f = Fixture::with_audit(audit.clone()).await;
    let issued = f.http.issue().await;
    let changed = save(
        &f,
        &issued,
        &body(&f, "# 当前版\n", "before-restore-audit"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    audit.mode.store(1, Ordering::Release);
    let input = restore(
        &f,
        &changed["receipt"]["new_revision"],
        "restore-audit-down",
    );
    assert_eq!(
        save(&f, &issued, &input, 503).await["code"],
        "audit_unavailable"
    );
    assert_eq!(f.journal()["committed_through"], 1);
    assert_eq!(
        fs::read_to_string(f.http.root.join("vault/a.md")).unwrap(),
        "# 当前版\n"
    );
    // 已持久取消证明没有恢复安装；不得连带禁止普通人编辑的审计降级。
    refresh(&f, &issued).await;
    let mut ordinary = body(&f, "# 审计降级的普通编辑\n", "normal-after-cancel");
    ordinary["expected_revision"] = changed["receipt"]["new_revision"].clone();
    save(&f, &issued, &ordinary, 200).await;
    f.close().await;
}

#[tokio::test]
async fn restore_outcome_outage_blocks_new_writes_until_the_same_request_is_confirmed() {
    let audit = Arc::new(SwitchAudit::default());
    let f = Fixture::with_audit(audit.clone()).await;
    let issued = f.http.issue().await;
    let changed = save(
        &f,
        &issued,
        &body(&f, "# 待恢复的新版\n", "before-outcome-down"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    let input = restore(
        &f,
        &changed["receipt"]["new_revision"],
        "restore-outcome-down",
    );
    audit.mode.store(2, Ordering::Release);
    assert_eq!(
        save(&f, &issued, &input, 503).await["code"],
        "outcome_unknown"
    );
    assert_eq!(f.journal()["committed_through"], 2);
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    // 同key不能换成普通replace来领取尚未确认的Restore回执。
    let mut disguised = body(
        &f,
        std::str::from_utf8(&f.expected.content).unwrap(),
        "restore-outcome-down",
    );
    disguised["expected_revision"] = changed["receipt"]["new_revision"].clone();
    assert_eq!(
        save(&f, &issued, &disguised, 409).await["code"],
        "idempotency_conflict"
    );
    let blocked = body(&f, "不允许安装", "write-during-restore");
    assert_eq!(
        save(&f, &issued, &blocked, 503).await["code"],
        "audit_unavailable"
    );
    assert_eq!(
        save(&f, &issued, &input, 503).await["code"],
        "outcome_unknown"
    );
    audit.mode.store(0, Ordering::Release);
    let replay = save(&f, &issued, &input, 200).await;
    assert_eq!(
        replay["receipt"]["new_revision"],
        f.expected.revision.to_hex()
    );
    assert_eq!(f.journal()["committed_through"], 2);
    let events = audit
        .query(&AuditFilter::default().with_action("recovery.page_restore"))
        .unwrap();
    assert_eq!(events.len(), 2);
    f.close().await;
}

#[tokio::test]
async fn restore_replay_does_not_need_source_bytes_or_reinstall_a_later_version() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let changed = save(
        &f,
        &issued,
        &body(&f, "# 先修改\n", "before-history-replay"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    let input = restore(
        &f,
        &changed["receipt"]["new_revision"],
        "restore-history-replay",
    );
    let original = save(&f, &issued, &input, 200).await;
    refresh(&f, &issued).await;
    save(
        &f,
        &issued,
        &body(&f, "# 后续另一版\n", "after-history-replay"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    // 模拟源历史字节不可读；不宣称这一步是正常的保留清理流程。
    fs::remove_file(f.data.join("history").join(f.expected.revision.to_hex())).unwrap();
    assert_eq!(save(&f, &issued, &input, 200).await, original);
    assert_eq!(
        fs::read_to_string(f.http.root.join("vault/a.md")).unwrap(),
        "# 后续另一版\n"
    );
    assert_eq!(f.journal()["committed_through"], 3);
    f.close().await;
}

#[tokio::test]
async fn restore_dto_cannot_smuggle_content_actor_or_a_bypass() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let input = restore(
        &f,
        &f.expected.revision.to_hex().into(),
        "restore-closed-body",
    );
    for field in ["content", "actor", "force", "ignore_audit"] {
        let mut invalid = input.clone();
        invalid[field] = "untrusted".into();
        save(&f, &issued, &invalid, 400).await;
    }
    assert_eq!(f.journal()["committed_through"], 0);
    f.close().await;
}
