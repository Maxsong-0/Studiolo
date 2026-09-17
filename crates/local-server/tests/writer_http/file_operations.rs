//! 真实Auth及CAS装配的合成临时文件闭环，不代替生产身份或远端同步验收。
use crate::{editing::refresh, fixture::Fixture, support};
use serde_json::{Value, json};
use std::fs;

pub(super) const PATH: &str = "/api/v1/file-operations";
pub(super) fn delete(f: &Fixture, key: &str) -> Value {
    json!({"kind":"delete","vault_uuid":f.expected.vault_uuid.to_string(),"path":"a.md",
        "expected_revision":f.expected.revision.to_hex(),"idempotency_key":key})
}
pub(super) async fn post(
    f: &Fixture,
    issued: &support::Issued,
    value: &Value,
    status: u16,
) -> Value {
    let mut request = f
        .http
        .request("POST", PATH, Some(&issued.cookie), &value.to_string());
    request
        .headers_mut()
        .insert("x-csrf-token", issued.csrf.parse().unwrap());
    support::json(f.http.send(request, status).await).await
}
pub(super) async fn page(
    f: &Fixture,
    issued: &support::Issued,
    suffix: &str,
    status: u16,
) -> Value {
    f.get(
        &format!(
            "/api/v1/tombstones?vault_uuid={}&{suffix}",
            f.expected.vault_uuid
        ),
        &issued.cookie,
        status,
    )
    .await
}

#[tokio::test]
async fn default_off_discovery_and_operations_do_not_initialize_typed_storage() {
    let f = Fixture::editable().await;
    let issued = f.http.issue().await;
    assert_eq!(f.get(PATH, &issued.cookie, 403).await["code"], "disabled");
    assert_eq!(
        post(&f, &issued, &delete(&f, "disabled-delete-01"), 403).await["code"],
        "disabled"
    );
    assert_eq!(page(&f, &issued, "limit=32", 403).await["code"], "disabled");
    assert_ne!(f.journal()["file_operations_enabled"], true);
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    f.close().await;
}

#[tokio::test]
async fn delete_list_restore_and_historical_replay_use_distinct_action_keys() {
    let f = Fixture::file_operations(None).await;
    let issued = f.http.issue().await;
    assert_eq!(
        f.get(PATH, &issued.cookie, 200).await,
        json!({"vault_uuid":f.expected.vault_uuid.to_string(),"enabled":true,"rename_enabled":false})
    );
    let input = delete(&f, "same-client-key-001");
    let deleted = post(&f, &issued, &input, 200).await;
    assert_eq!(deleted["receipt"]["sequence"], "1");
    assert_eq!(
        deleted["receipt"]["effects"][0]["before"]["revision"],
        f.expected.revision.to_hex()
    );
    assert_eq!(
        deleted["receipt"]["effects"][0]["after"],
        json!({"kind":"missing"})
    );
    assert!(!f.http.root.join("vault/a.md").exists());
    refresh(&f, &issued).await;
    let listing = page(&f, &issued, "limit=32", 200).await;
    assert_eq!(listing["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        listing["entries"][0]["revision"],
        f.expected.revision.to_hex()
    );
    assert_eq!(
        listing["entries"][0]["deleted_by"]["subject"],
        "fictional-person"
    );
    let restore = json!({"kind":"restore_tombstone","vault_uuid":f.expected.vault_uuid.to_string(),
        "path":"a.md","tombstone_id":deleted["receipt"]["tombstone_id"],"idempotency_key":"same-client-key-001"});
    let restored = post(&f, &issued, &restore, 200).await;
    assert_ne!(
        restored["receipt"]["operation_id"],
        deleted["receipt"]["operation_id"]
    );
    assert_eq!(
        restored["receipt"]["effects"][0]["before"],
        json!({"kind":"missing"})
    );
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    assert_eq!(post(&f, &issued, &input, 200).await, deleted);
    assert_eq!(post(&f, &issued, &restore, 200).await, restored);
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    f.close().await;
}

#[tokio::test]
async fn fixed_delete_rejection_survives_missing_aba_and_restore_never_overwrites() {
    let f = Fixture::file_operations(None).await;
    let issued = f.http.issue().await;
    let input = delete(&f, "fixed-rejected-001");
    let path = f.http.root.join("vault/a.md");
    fs::remove_file(&path).unwrap();
    refresh(&f, &issued).await;
    assert_eq!(
        post(&f, &issued, &input, 409).await["code"],
        "file_operation_conflict"
    );
    fs::write(&path, &f.expected.content).unwrap();
    refresh(&f, &issued).await;
    assert_eq!(
        post(&f, &issued, &input, 409).await["code"],
        "file_operation_conflict"
    );
    let deleted = post(&f, &issued, &delete(&f, "fresh-delete-001"), 200).await;
    fs::write(&path, b"external replacement").unwrap();
    refresh(&f, &issued).await;
    let restore = json!({"kind":"restore_tombstone","vault_uuid":f.expected.vault_uuid.to_string(),"path":"a.md",
        "tombstone_id":deleted["receipt"]["tombstone_id"],"idempotency_key":"fixed-restore-001"});
    assert_eq!(
        post(&f, &issued, &restore, 409).await["code"],
        "file_operation_conflict"
    );
    assert_eq!(fs::read(&path).unwrap(), b"external replacement");
    fs::remove_file(&path).unwrap();
    refresh(&f, &issued).await;
    assert_eq!(
        post(&f, &issued, &restore, 409).await["code"],
        "file_operation_conflict"
    );
    assert!(!path.exists());
    f.close().await;
}

#[tokio::test]
async fn auth_csrf_closed_dto_and_registered_generated_whole_file_boundary_hold() {
    let f = Fixture::file_operations(None).await;
    let issued = f.http.issue().await;
    let input = delete(&f, "guard-delete-0001");
    f.http
        .send(
            f.http
                .request("POST", PATH, Some(&issued.cookie), &input.to_string()),
            403,
        )
        .await;
    for field in ["actor", "scope_hash", "precondition", "force"] {
        let mut bad = input.clone();
        bad[field] = true.into();
        assert_eq!(
            post(&f, &issued, &bad, 400).await["code"],
            "invalid_file_operation"
        );
    }
    for path in ["mixed.md", "CLAUDE.md", "private.md"] {
        let mut bad = input.clone();
        bad["path"] = path.into();
        assert_eq!(post(&f, &issued, &bad, 403).await["code"], "protected");
    }
    // 即使生成哨兵已经被外部移除，原配置的整文件排除仍然有效。
    fs::write(f.http.root.join("vault/mixed.md"), "普通文本").unwrap();
    let mut mixed = input.clone();
    mixed["path"] = "mixed.md".into();
    assert_eq!(post(&f, &issued, &mixed, 403).await["code"], "protected");
    assert!(
        page(&f, &issued, "limit=32", 200).await["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(f.journal()["committed_through"], 0);
    f.close().await;
}
