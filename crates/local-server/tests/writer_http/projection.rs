use crate::{fixture::Fixture, support};
use serde_json::Value;

#[tokio::test]
async fn writer_snapshot_preserves_decimal_watermark_and_real_canonical_revision() {
    let watermark = 9_007_199_254_740_993_u64;
    let f = Fixture::new(watermark, false).await;
    let issued = f.http.issue().await;
    let snapshot = f.get("/api/workbench", &issued.cookie, 200).await;
    let fresh = &snapshot["freshness"];
    assert_eq!(fresh["stale"], false);
    assert_eq!(fresh["watcher_healthy"], true);
    assert_eq!(fresh["unresolved"], false);
    for field in ["indexed_through", "committed_through"] {
        assert_eq!(fresh[field], watermark.to_string());
    }
    let file = f.get("/api/v1/file?path=a.md", &issued.cookie, 200).await;
    assert_eq!(file.as_object().unwrap().len(), 4);
    assert_eq!(file["vault_uuid"], f.expected.vault_uuid.to_string());
    assert_eq!(file["path"], "a.md");
    assert_eq!(
        file["content"],
        String::from_utf8(f.expected.content.clone()).unwrap()
    );
    assert_eq!(file["revision"], f.expected.revision.to_hex());
    assert!(file.get("generation").is_none());
    let generation = snapshot["generation"].as_u64().unwrap();
    let document = f
        .get(
            &format!("/api/document?path=a.md&generation={generation}"),
            &issued.cookie,
            200,
        )
        .await;
    assert_eq!(document["content"], file["content"]);
    for uri in [
        format!("/api/tree?path=&generation={generation}"),
        format!("/api/search?q=reading&mode=full_text&generation={generation}"),
    ] {
        let body = f.get(&uri, &issued.cookie, 200).await;
        assert_eq!(body["generation"], generation);
    }
    let refreshed = f
        .get("/api/workbench?refresh=true", &issued.cookie, 200)
        .await;
    assert!(refreshed["generation"].as_u64().unwrap() > generation);
    assert_eq!(refreshed["freshness"], snapshot["freshness"]);
    let encoded = snapshot.to_string();
    assert!(!encoded.contains(f.http.root.to_str().unwrap()));
    assert!(!encoded.contains(&issued.token));
    assert!(!encoded.contains("pair_id"));
    f.close().await;
}

#[tokio::test]
async fn writer_read_budget_is_exact_and_ordinary_rejections_keep_view_fresh() {
    let f = Fixture::new(0, true).await;
    let issued = f.http.issue().await;
    let response = f
        .http
        .send(
            f.http.request(
                "GET",
                "/api/v1/file?path=exact.md",
                Some(&issued.cookie),
                "",
            ),
            200,
        )
        .await;
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["content"].as_str().unwrap().len(), 1024 * 1024);
    for (uri, status) in [
        ("/api/v1/file?path=over.md", 413),
        ("/api/v1/file?path=missing.md", 404),
        ("/api/v1/file?path=.workbench/private.md", 403),
        ("/api/v1/file?path=a.md&root=/tmp", 400),
    ] {
        f.get(uri, &issued.cookie, status).await;
    }
    assert_eq!(
        f.get("/api/workbench", &issued.cookie, 200).await["freshness"]["stale"],
        false
    );
    f.close().await;
}

#[tokio::test]
async fn writer_entry_preserves_authentication_and_has_no_mutation_route() {
    let f = Fixture::new(0, false).await;
    f.http
        .send(
            f.http.request("GET", "/api/v1/file?path=a.md", None, ""),
            401,
        )
        .await;
    let issued = f.http.issue().await;
    for header in [
        "cf-access-jwt-assertion",
        "x-studiolo-request",
        "sec-fetch-site",
    ] {
        let mut request = f
            .http
            .request("GET", "/api/v1/file?path=a.md", Some(&issued.cookie), "");
        request.headers_mut().remove(header);
        f.http
            .send(
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
        f.http
            .send(
                f.http
                    .request(method, "/api/v1/file?path=a.md", Some(&issued.cookie), "{}"),
                405,
            )
            .await;
    }
    f.close().await;
    let legacy = support::Fixture::new(true).await;
    let issued = legacy.issue().await;
    let body = support::json(
        legacy
            .send(
                legacy.request("GET", "/api/workbench", Some(&issued.cookie), ""),
                200,
            )
            .await,
    )
    .await;
    assert!(body.get("freshness").is_none());
}
