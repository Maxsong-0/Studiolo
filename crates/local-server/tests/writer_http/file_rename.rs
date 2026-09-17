//! 真实会话、私有策略、唯一 Broker 与 SQLite 审计；只使用合成临时文件。
#[path = "file_rename_content.rs"]
mod content;
#[path = "file_operation_reopen.rs"]
mod legacy_reopen;
#[path = "file_rename_support.rs"]
mod support;
use crate::{
    file_operations::{PATH, delete, page, post},
    fixture::Fixture,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, sync::Arc};
use studiolo_audit_ledger::AuditPhase;
use studiolo_core::{Revision, VaultPath};
use studiolo_host_config::Config;

const TO: &str = "files/renamed.md";
fn rename(f: &Fixture, key: &str) -> Value {
    json!({"kind":"rename","vault_uuid":f.expected.vault_uuid,"from":"a.md","to":TO,
        "expected_revision":f.expected.revision.to_hex(),"idempotency_key":key})
}
fn unchanged(f: &Fixture, before: &Value, content: &[u8]) {
    assert_eq!(f.journal(), *before, "拒绝不能消耗序号或生成 typed 槽");
    assert_eq!(fs::read(f.http.root.join("vault/a.md")).unwrap(), content);
    assert!(!f.http.root.join("vault").join(TO).exists());
    assert!(f.http.events("vault.mutation").is_empty());
}
fn committed(f: &Fixture, result: &Value) {
    assert_eq!(result["status"], "committed");
    let receipt = &result["receipt"];
    assert_eq!(receipt["sequence"], "1");
    let id = receipt["operation_id"].as_str().unwrap();
    assert_eq!(id.len(), 64);
    assert!(
        id.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(receipt["outcome"], "committed");
    assert_eq!(receipt["tombstone_id"], receipt["operation_id"]);
    assert!(receipt["evidence_id"].is_null());
    let effects = receipt["effects"].as_array().unwrap();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0]["kind"], "confirmed");
    assert_eq!(effects[0]["path"], TO);
    assert_eq!(effects[0]["before"], json!({"kind":"missing"}));
    assert_eq!(effects[1]["kind"], "confirmed");
    assert_eq!(effects[1]["path"], "a.md");
    assert_eq!(effects[1]["after"], json!({"kind":"missing"}));
    let target = &effects[0]["after"];
    let source = &effects[1]["before"];
    assert_eq!(source["kind"], "present");
    assert_eq!(target["kind"], "present");
    assert_eq!(source["revision"], f.expected.revision.to_hex());
    assert_eq!(
        target["revision"],
        Revision::compute(
            f.expected.vault_uuid,
            &VaultPath::parse(TO).unwrap(),
            &f.expected.content
        )
        .to_hex()
    );
    assert_ne!(source["revision"], target["revision"]);
    assert_eq!(source["bytes"], f.expected.content.len() as u64);
    assert_eq!(source["bytes"], target["bytes"]);
    assert_eq!(source["sha256"], target["sha256"]);
    let raw: String = Sha256::digest(&f.expected.content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(source["sha256"], raw);
    assert!(!f.http.root.join("vault/a.md").exists());
    assert_eq!(
        fs::read(f.http.root.join("vault").join(TO)).unwrap(),
        f.expected.content
    );
    for private in [
        "content",
        "writer-data",
        ".workbench",
        "stage",
        "scope_hash",
    ] {
        assert!(!result.to_string().contains(private));
    }
}

#[tokio::test]
async fn approved_rename_preserves_bytes_two_effects_original_key_and_reopen() {
    let mut f =
        Fixture::file_rename(None, "\u{feff}# 中文\r\n\t正文\u{feff}\n".as_bytes(), false).await;
    let issued = f.http.issue().await;
    assert_eq!(
        f.get(PATH, &issued.cookie, 200).await,
        json!({"vault_uuid":f.expected.vault_uuid,"enabled":true,"rename_enabled":true})
    );
    let input = rename(&f, "rename-http-original-01");
    let saved = post(&f, &issued, &input, 200).await;
    committed(&f, &saved);
    // 源已经 Missing；原键回放不得先做正文读取或换成新的 CAS。
    assert_eq!(post(&f, &issued, &input, 200).await, saved);
    for (field, value) in [
        ("to", json!("files/other.md")),
        (
            "expected_revision",
            json!(
                Revision::compute(
                    f.expected.vault_uuid,
                    &VaultPath::parse("a.md").unwrap(),
                    b"other"
                )
                .to_hex()
            ),
        ),
    ] {
        let mut changed = input.clone();
        changed[field] = value;
        assert_eq!(
            post(&f, &issued, &changed, 409).await["code"],
            "idempotency_conflict"
        );
    }
    f.reopen_file_operations().await;
    assert_eq!(post(&f, &issued, &input, 200).await, saved);
    committed(&f, &saved);
    let tombstones = page(&f, &issued, "limit=32", 200).await;
    assert_eq!(tombstones["entries"].as_array().unwrap().len(), 1);
    assert_eq!(tombstones["entries"][0]["path"], "a.md");
    assert_eq!(
        tombstones["entries"][0]["id"],
        saved["receipt"]["tombstone_id"]
    );
    let records = f.http.events("vault.mutation");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    f.close().await;
}

#[tokio::test]
async fn ordinary_file_operations_do_not_enable_rename_or_consume_a_sequence() {
    let f = Fixture::file_operations(None).await;
    let issued = f.http.issue().await;
    assert_eq!(
        f.get(PATH, &issued.cookie, 200).await["rename_enabled"],
        false
    );
    let before = f.journal();
    assert_ne!(before["rename_enabled"], true);
    assert_eq!(
        post(&f, &issued, &rename(&f, "rename-disabled-001"), 403).await["code"],
        "disabled"
    );
    unchanged(&f, &before, &f.expected.content);
    // 关闭 Rename 并未关闭原先明确批准的 Delete。
    post(&f, &issued, &delete(&f, "old-delete-still-enabled"), 200).await;
    f.close().await;
}

#[tokio::test]
async fn csrf_closed_request_and_both_path_policy_reject_before_mutation() {
    let f = Fixture::file_rename(None, b"private text\n", false).await;
    let issued = f.http.issue().await;
    let input = rename(&f, "rename-guards-0001");
    let before = f.journal();
    f.http
        .send(
            f.http
                .request("POST", PATH, Some(&issued.cookie), &input.to_string()),
            403,
        )
        .await;
    for field in [
        "path",
        "content",
        "class",
        "mime",
        "overwrite",
        "scope_hash",
    ] {
        let mut bad = input.clone();
        bad[field] = true.into();
        assert_eq!(
            post(&f, &issued, &bad, 400).await["code"],
            "invalid_file_operation"
        );
    }
    for (field, value) in [
        ("from", "private.md"),
        ("to", "private.md"),
        ("to", "files/unregistered.md"),
        ("to", "files/renamed.MD"),
    ] {
        let mut bad = input.clone();
        bad[field] = value.into();
        assert_eq!(post(&f, &issued, &bad, 403).await["code"], "protected");
    }
    unchanged(&f, &before, &f.expected.content);
    f.close().await;
}

#[tokio::test]
async fn sync_only_generated_registration_protects_delete_restore_and_both_rename_paths() {
    let f = Fixture::file_rename(None, b"ordinary human file\n", true).await;
    let issued = f.http.issue().await;
    let config = Config::load(&f.http.root.join("config/owner.json")).unwrap();
    let policy = config.file_operation_policy().unwrap().unwrap();
    let path = VaultPath::parse("files/generated.md").unwrap();
    let roots = config
        .canonical_serving_roots(
            &f.http.root.join("vault"),
            &f.http.root.join("web"),
            &f.data,
        )
        .unwrap();
    assert!(
        roots
            .human_content()
            .unwrap()
            .allows(f.expected.vault_uuid, &path)
    );
    assert!(!policy.allows(f.expected.vault_uuid, &path).unwrap());
    let generated = fs::read(f.http.root.join("vault/files/generated.md")).unwrap();
    let before = f.journal();
    for name in ["files/generated.md", "files/GENERATED.md"] {
        let mut remove = delete(&f, "generated-delete-001");
        remove["path"] = name.into();
        remove["expected_revision"] = Revision::compute(f.expected.vault_uuid, &path, &generated)
            .to_hex()
            .into();
        let restore = json!({"kind":"restore_tombstone","vault_uuid":f.expected.vault_uuid,
            "path":name,"tombstone_id":"0".repeat(64),"idempotency_key":"generated-restore-001"});
        let mut from = rename(&f, "generated-rename-from");
        from["from"] = name.into();
        let mut to = rename(&f, "generated-rename-to");
        to["to"] = name.into();
        for denied in [remove, restore, from, to] {
            assert_eq!(post(&f, &issued, &denied, 403).await["code"], "protected");
        }
    }
    unchanged(&f, &before, &f.expected.content);
    assert_eq!(
        fs::read(f.http.root.join("vault/files/generated.md")).unwrap(),
        generated
    );
    assert!(
        page(&f, &issued, "limit=32", 200).await["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    f.close().await;
}

#[tokio::test]
async fn sqlite_outcome_failure_reopens_and_only_replays_the_original_pending_receipt() {
    let mut f = Fixture::file_rename(None, b"pending original bytes\n", false).await;
    let audit = Arc::new(support::OutcomeFailure::open(&f.http.audit));
    f.reopen_file_operations_with_audit(Some(audit)).await;
    let issued = f.http.issue().await;
    let input = rename(&f, "rename-pending-reopen");
    assert_eq!(
        post(&f, &issued, &input, 503).await["code"],
        "outcome_unknown"
    );
    let pending = f.journal();
    assert_eq!(pending["file_operation"]["phase"], "finalized_await_audit");
    assert!(!f.http.root.join("vault/a.md").exists());
    assert_eq!(
        fs::read(f.http.root.join("vault").join(TO)).unwrap(),
        f.expected.content
    );
    let fixed = &pending["file_operation"]["terminal"]["receipt"];
    assert_eq!(fixed["outcome"], "committed");
    let records = f.http.events("vault.mutation");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    // 新 owner 复用真实 SQLite 原 Intent 账本；不靠内存 ledger 抹掉持久 ACK。
    f.reopen_file_operations().await;
    let replay = post(&f, &issued, &input, 200).await;
    committed(&f, &replay);
    assert_eq!(replay["receipt"]["operation_id"], fixed["operation_id"]);
    assert_eq!(post(&f, &issued, &input, 200).await, replay);
    assert!(f.journal()["file_operation"].is_null());
    let records = f.http.events("vault.mutation");
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    f.close().await;
}
