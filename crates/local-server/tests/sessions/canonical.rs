//! 同一个临时canonical通过M0初始化，再由新窄读入口提供真实revision。

use super::{support::*, test_support::verifier};
use std::fs;
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{CanonicalReadVault, FsVaultBroker, ReadOnlyVault};

async fn fixture() -> Fixture {
    let mut f = Fixture::new(true).await;
    drop(FsVaultBroker::open(&f.root.join("vault"), &f.root.join("data")).unwrap());
    f.app = canonical_router(&f, &f.root.join("vault")).await.unwrap();
    f
}

async fn canonical_router(
    f: &Fixture,
    canonical_root: &std::path::Path,
) -> Result<axum::Router, studiolo_local_server::LocalError> {
    studiolo_local_server::router_with_canonical_sessions(
        (
            ReadOnlyVault::open(&f.root.join("vault")).unwrap(),
            std::sync::Arc::new(CanonicalReadVault::open_existing(canonical_root).unwrap()),
        ),
        ReadOnlyVault::open(&f.root.join("web")).unwrap(),
        "虚构canonical".into(),
        4319,
        verifier().await,
        f.manager(),
    )
}

#[tokio::test]
async fn canonical_file_returns_same_bytes_and_revision_as_broker_and_survives_reopen() {
    let mut f = fixture().await;
    let issued = f.issue().await;
    let expected = CanonicalReadVault::open_existing(&f.root.join("vault")).unwrap();
    let note = expected
        .read_file(&VaultPath::parse("a.md").unwrap(), 1024)
        .unwrap();
    let body = json(
        f.send(
            f.request("GET", "/api/v1/file?path=a.md", Some(&issued.cookie), ""),
            200,
        )
        .await,
    )
    .await;
    assert_eq!(body.as_object().unwrap().len(), 6);
    assert_eq!(body["editing"]["allowed"], false);
    assert_eq!(body["content"], String::from_utf8(note.content).unwrap());
    assert_eq!(body["path"], "a.md");
    assert_eq!(body["vault_uuid"], note.vault_uuid.to_string());
    assert_eq!(body["revision"], note.revision.to_hex());
    assert!(body.get("generation").is_none());
    f.app = axum::Router::new();
    f.app = canonical_router(&f, &f.root.join("vault")).await.unwrap();
    let reopened = json(
        f.send(
            f.request("GET", "/api/v1/file?path=a.md", Some(&issued.cookie), ""),
            200,
        )
        .await,
    )
    .await;
    assert_eq!(body, reopened);
}

#[tokio::test]
async fn canonical_read_observes_external_change_without_claiming_index_generation_is_current() {
    let f = fixture().await;
    let issued = f.issue().await;
    let root = CanonicalReadVault::open_existing(&f.root.join("vault")).unwrap();
    let content = "# 新正文\n<script>text only</script>\n";
    fs::write(f.root.join("vault/a.md"), content).unwrap();
    let body = json(
        f.send(
            f.request("GET", "/api/v1/file?path=a.md", Some(&issued.cookie), ""),
            200,
        )
        .await,
    )
    .await;
    assert_eq!(body["content"], content);
    assert_eq!(
        body["revision"],
        Revision::compute(
            root.vault_uuid(),
            &VaultPath::parse("a.md").unwrap(),
            content.as_bytes()
        )
        .to_hex()
    );
    let indexed = json(
        f.send(
            f.request(
                "GET",
                "/api/document?path=a.md&generation=1",
                Some(&issued.cookie),
                "",
            ),
            200,
        )
        .await,
    )
    .await;
    assert_ne!(indexed["content"], content);
}

#[tokio::test]
async fn canonical_file_requires_full_session_and_legacy_mode_does_not_expose_it() {
    let f = fixture().await;
    f.send(f.request("GET", "/api/v1/file?path=a.md", None, ""), 401)
        .await;
    let issued = f.issue().await;
    for header in [
        "cf-access-jwt-assertion",
        "x-studiolo-request",
        "sec-fetch-site",
    ] {
        let mut request = f.request("GET", "/api/v1/file?path=a.md", Some(&issued.cookie), "");
        request.headers_mut().remove(header);
        f.send(
            request,
            if header == "cf-access-jwt-assertion" {
                401
            } else {
                403
            },
        )
        .await;
    }
    for method in ["HEAD", "POST", "PATCH", "DELETE"] {
        f.send(
            f.request(method, "/api/v1/file?path=a.md", Some(&issued.cookie), "{}"),
            405,
        )
        .await;
    }
    let legacy = Fixture::new(true).await;
    let old = legacy.issue().await;
    legacy
        .send(
            legacy.request("GET", "/api/v1/file?path=a.md", Some(&old.cookie), ""),
            404,
        )
        .await;
}

#[tokio::test]
async fn canonical_query_is_closed_bounded_and_protected_paths_are_rejected() {
    let f = fixture().await;
    let issued = f.issue().await;
    fs::write(f.root.join("vault/UPPER.MD"), b"# uppercase\n").unwrap();
    let uppercase = json(
        f.send(
            f.request(
                "GET",
                "/api/v1/file?path=UPPER.MD",
                Some(&issued.cookie),
                "",
            ),
            200,
        )
        .await,
    )
    .await;
    assert_eq!(uppercase["content"], "# uppercase\n");
    for query in [
        "",
        "path=",
        "path=a.md&path=b.md",
        "path=a.md&root=/tmp",
        "path=../a.md",
        "path=/a.md",
        "path=a.pdf",
        "path=a.md%00",
    ] {
        f.send(
            f.request(
                "GET",
                &format!("/api/v1/file?{query}"),
                Some(&issued.cookie),
                "",
            ),
            400,
        )
        .await;
    }
    f.send(
        f.request(
            "GET",
            "/api/v1/file?path=.workbench/private.md",
            Some(&issued.cookie),
            "",
        ),
        403,
    )
    .await;
    f.send(
        f.request(
            "GET",
            "/api/v1/file?path=missing.md",
            Some(&issued.cookie),
            "",
        ),
        404,
    )
    .await;
    fs::write(f.root.join("vault/large.md"), vec![b'a'; 1024 * 1024 + 1]).unwrap();
    f.send(
        f.request(
            "GET",
            "/api/v1/file?path=large.md",
            Some(&issued.cookie),
            "",
        ),
        413,
    )
    .await;
    fs::write(f.root.join("vault/invalid.md"), [0xff]).unwrap();
    f.send(
        f.request(
            "GET",
            "/api/v1/file?path=invalid.md",
            Some(&issued.cookie),
            "",
        ),
        422,
    )
    .await;
}

#[tokio::test]
async fn canonical_identity_change_fails_closed_and_different_root_cannot_be_composed() {
    let f = fixture().await;
    let issued = f.issue().await;
    let other = f.root.join("other");
    drop(FsVaultBroker::open(&other, &f.root.join("other-data")).unwrap());
    assert!(matches!(
        canonical_router(&f, &other).await,
        Err(studiolo_local_server::LocalError::Configuration)
    ));
    fs::copy(
        other.join(".workbench/vault.json"),
        f.root.join("vault/.workbench/vault.json"),
    )
    .unwrap();
    let response = f
        .send(
            f.request("GET", "/api/v1/file?path=a.md", Some(&issued.cookie), ""),
            503,
        )
        .await;
    let body = json(response).await;
    assert!(!body.to_string().contains(f.root.to_str().unwrap()));
    assert!(body.get("content").is_none());
}
