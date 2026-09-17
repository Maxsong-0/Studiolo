//! 过滤固定页面而非无限填页；配置重新批准后，旧条目的私有路径不再出现在返回中。
use crate::{
    editing::refresh,
    file_operations::{PATH, delete, page, post},
    fixture::Fixture,
};
use serde_json::{Value, json};
use std::fs;
use studiolo_core::{Revision, VaultPath};

#[tokio::test]
async fn restart_discovery_and_empty_filtered_page_preserve_the_original_cursor() {
    let mut f = Fixture::file_operations(None).await;
    let issued = f.http.issue().await;
    fs::create_dir(f.http.root.join("vault/files")).unwrap();
    for name in ["files/first.md", "files/second.md", "files/third.md"] {
        let path = VaultPath::parse(name).unwrap();
        fs::write(f.http.root.join("vault").join(name), b"temporary page").unwrap();
        refresh(&f, &issued).await;
        let input = json!({"kind":"delete","vault_uuid":f.expected.vault_uuid,"path":name,
            "expected_revision":Revision::compute(f.expected.vault_uuid,&path,b"temporary page").to_hex(),
            "idempotency_key":format!("delete-fixture-{}",name.replace(['/', '.'], "_"))});
        post(&f, &issued, &input, 200).await;
        refresh(&f, &issued).await;
    }
    let first = page(&f, &issued, "limit=1", 200).await;
    let hidden = first["entries"][0]["path"].as_str().unwrap().to_owned();
    let cursor = first["next_cursor"].as_str().unwrap().to_owned();
    f.owner.shutdown().await.unwrap();
    let file = f.http.root.join("config/owner.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    config["human_content"]["protected"] = json!([hidden]);
    fs::write(&file, serde_json::to_vec(&config).unwrap()).unwrap();
    f.reopen_file_operations().await;
    assert_eq!(
        f.get(PATH, &issued.cookie, 200).await["vault_uuid"],
        f.expected.vault_uuid.to_string()
    );
    let filtered = page(&f, &issued, "limit=1", 200).await;
    assert!(filtered["entries"].as_array().unwrap().is_empty());
    assert_eq!(filtered["next_cursor"], cursor);
    assert!(!filtered.to_string().contains(&hidden));
    assert!(!filtered.to_string().contains("deleted_by"));
    let second = page(&f, &issued, &format!("limit=1&cursor={cursor}"), 200).await;
    assert_eq!(second["entries"].as_array().unwrap().len(), 1);
    assert_ne!(second["entries"][0]["path"], hidden);
    post(&f, &issued, &delete(&f, "change-tombstone-generation"), 200).await;
    assert_eq!(
        page(&f, &issued, &format!("limit=1&cursor={cursor}"), 409).await["code"],
        "tombstones_changed"
    );
    f.close().await;
}

#[tokio::test]
async fn list_query_is_bounded_and_never_accepts_a_foreign_vault_binding() {
    let f = Fixture::file_operations(None).await;
    let issued = f.http.issue().await;
    for suffix in [
        "limit=0",
        "limit=65",
        "limit=1&limit=2",
        "limit=1&unknown=true",
        "cursor=",
    ] {
        assert_eq!(
            page(&f, &issued, suffix, 400).await["code"],
            "invalid_tombstone_query"
        );
    }
    let uri = format!(
        "/api/v1/tombstones?vault_uuid={}",
        studiolo_core::VaultUuid::new()
    );
    assert_eq!(f.get(&uri, &issued.cookie, 403).await["code"], "protected");
    f.close().await;
}
