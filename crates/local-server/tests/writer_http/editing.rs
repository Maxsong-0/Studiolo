//! 真实Auth、writer和持久历史经HTTP闭环；不直接把文件写入当保存成功。
use crate::{fixture::Fixture, support};
use serde_json::{Value, json};

pub(super) fn body(f: &Fixture, content: &str, key: &str) -> Value {
    json!({"kind":"replace","vault_uuid":f.expected.vault_uuid.to_string(),"path":"a.md",
        "expected_revision":f.expected.revision.to_hex(),"content":content,"idempotency_key":key})
}
pub(super) async fn save(
    f: &Fixture,
    issued: &support::Issued,
    body: &Value,
    status: u16,
) -> Value {
    let mut request = f.http.request(
        "POST",
        "/api/v1/mutations",
        Some(&issued.cookie),
        &body.to_string(),
    );
    request
        .headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    support::json(f.http.send(request, status).await).await
}
pub(super) async fn refresh(f: &Fixture, issued: &support::Issued) {
    f.get("/api/workbench?refresh=true", &issued.cookie, 200)
        .await;
}

#[tokio::test]
async fn trusted_scope_saves_with_real_cas_and_returns_historical_receipt() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let initial = f.get("/api/v1/file?path=a.md", &issued.cookie, 200).await;
    assert_eq!(initial["editing"]["allowed"], true);
    let input = body(&f, "# 新正文\n可持久恢复。\n", "save-contract-001");
    let saved = save(&f, &issued, &input, 200).await;
    assert_eq!(saved["status"], "committed");
    assert_eq!(
        saved["receipt"]["previous_revision"],
        f.expected.revision.to_hex()
    );
    refresh(&f, &issued).await;
    let current = f.get("/api/v1/file?path=a.md", &issued.cookie, 200).await;
    assert_eq!(current["content"], input["content"]);
    assert_eq!(current["revision"], saved["receipt"]["new_revision"]);
    assert_eq!(save(&f, &issued, &input, 200).await, saved);
    f.close().await;
}

#[tokio::test]
async fn two_editors_receive_conflict_without_losing_either_draft_or_disk() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    save(
        &f,
        &issued,
        &body(&f, "# 第一位编辑者\n", "save-editor-first"),
        200,
    )
    .await;
    refresh(&f, &issued).await;
    let second = body(&f, "# 第二位本地稿\n", "save-editor-second");
    let conflict = save(&f, &issued, &second, 409).await;
    assert_eq!(conflict["code"], "revision_conflict");
    assert!(conflict.get("content").is_none());
    assert_eq!(
        f.get("/api/v1/file?path=a.md", &issued.cookie, 200).await["content"],
        "# 第一位编辑者\n"
    );
    refresh(&f, &issued).await;
    assert_eq!(save(&f, &issued, &second, 409).await, conflict);
    f.close().await;
}

#[tokio::test]
async fn ordinary_l2_save_requires_csrf_origin_and_closed_dto() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let input = body(&f, "draft", "save-guard-boundary");
    f.http
        .send(
            f.http.request(
                "POST",
                "/api/v1/mutations",
                Some(&issued.cookie),
                &input.to_string(),
            ),
            403,
        )
        .await;
    for field in ["force", "actor", "ignore_stale", "precondition"] {
        let mut changed = input.clone();
        changed[field] = true.into();
        save(&f, &issued, &changed, 400).await;
    }
    let mut request = f.http.request(
        "POST",
        "/api/v1/mutations",
        Some(&issued.cookie),
        &input.to_string(),
    );
    request
        .headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    request.headers_mut().remove("origin");
    f.http.send(request, 403).await;
    assert_eq!(f.journal()["committed_through"], 0);
    f.close().await;
}

#[tokio::test]
async fn protected_policy_and_generated_bytes_are_not_regular_content() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let mut input = body(&f, "changed", "save-protected-path");
    input["path"] = "CLAUDE.md".into();
    save(&f, &issued, &input, 403).await;
    let mixed = f
        .get("/api/v1/file?path=mixed.md", &issued.cookie, 200)
        .await;
    input["path"] = "mixed.md".into();
    input["expected_revision"] = mixed["revision"].clone();
    input["content"] = mixed["content"]
        .as_str()
        .unwrap()
        .replace("机器区", "篡改")
        .into();
    save(&f, &issued, &input, 403).await;
    input["content"] = mixed["content"]
        .as_str()
        .unwrap()
        .replace("正文", "人修改正文")
        .into();
    save(&f, &issued, &input, 200).await;
    f.close().await;
}

#[tokio::test]
async fn malformed_yaml_or_cross_vault_never_reaches_broker_installation() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    let invalid = body(&f, "---\nbroken: [\n---\nbody", "save-invalid-yaml");
    assert_eq!(
        save(&f, &issued, &invalid, 422).await["code"],
        "invalid_document"
    );
    let mut wrong = body(&f, "content", "save-other-vault");
    wrong["vault_uuid"] = studiolo_core::VaultUuid::new().to_string().into();
    save(&f, &issued, &wrong, 403).await;
    assert_eq!(f.journal()["committed_through"], 0);
    f.close().await;
}
