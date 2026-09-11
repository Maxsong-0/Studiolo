use super::support::{denied, fixture};
use reqwest::Method;

const ROUTES: [&str; 4] = [
    "/read/handshake",
    "/read/entries?project=School",
    "/read/note?path=School%2Fsecret.md",
    "/read/search?project=School&text=secret",
];

#[tokio::test]
async fn methods_and_malformed_queries_are_rejected_with_safe_headers() {
    let (_temp, f) = fixture().await;
    let (session, _, _) = f.handshake().await;
    for method in [
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::HEAD,
        Method::OPTIONS,
    ] {
        for route in ROUTES {
            denied(
                f.request(method.clone(), route, Some(&f.token), Some(&session))
                    .header("Origin", "https://untrusted.example")
                    .send()
                    .await
                    .unwrap(),
                405,
            )
            .await;
        }
    }
    for route in [
        "/read/handshake?extra=x",
        "/read/entries",
        "/read/entries?project=x&project=y",
        "/read/entries?project=x&extra=y",
        "/read/note",
        "/read/note?path=x&path=y",
        "/read/note?path=x&extra=y",
        "/read/note?path=%FF",
        "/read/note?path=%ZZ",
        "/read/search?project=x",
        "/read/search?text=x",
        "/read/search?project=x&text=x&text=y",
        "/read/search?project=x&text=x&mode=path&mode=full_text",
        "/read/search?project=x&text=x&extra=y",
    ] {
        denied(f.get(route, Some(&session)).send().await.unwrap(), 400).await;
    }
    for route in ROUTES {
        denied(
            f.get(route, Some(&session))
                .header("Studiolo-Session-Id", &session)
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
    }
    for session in ["", "unknown-session", "0000000000000000000000000!"] {
        denied(
            f.get("/read/handshake", Some(session))
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
    }
}

#[tokio::test]
async fn paths_and_search_modes_follow_the_same_whitelist_contract_as_mcp() {
    let (_temp, f) = fixture().await;
    let (session, _, _) = f.handshake().await;
    for (path, status) in [
        ("School/secret.md", 400),
        ("项目/Beta/00-总览/INDEX.md", 400),
        ("../School/secret.md", 400),
        ("/项目/Alpha/00-总览/INDEX.md", 400),
        ("项目/Alpha/../Beta/00-总览/INDEX.md", 400),
        ("项目/Alpha/missing.md", 404),
    ] {
        denied(
            f.get("/read/note", Some(&session))
                .query(&[("path", path)])
                .send()
                .await
                .unwrap(),
            status,
        )
        .await;
    }
    for project in ["School", "项目/Beta", "项目/Alpha/00-总览", "../项目/Alpha"] {
        denied(
            f.get("/read/entries", Some(&session))
                .query(&[("project", project)])
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
        denied(
            f.get("/read/search", Some(&session))
                .query(&[("project", project), ("text", "secret")])
                .send()
                .await
                .unwrap(),
            400,
        )
        .await;
    }
    denied(
        f.get("/read/search", Some(&session))
            .query(&[
                ("project", "项目/Alpha"),
                ("text", "Alpha"),
                ("mode", "regex"),
            ])
            .send()
            .await
            .unwrap(),
        400,
    )
    .await;
    let path_search = f
        .get("/read/search", Some(&session))
        .query(&[
            ("project", "项目/Alpha"),
            ("text", "00-总览 INDEX"),
            ("mode", "path"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(path_search.status(), 200);
    super::support::assert_headers(&path_search, true);
    assert!(
        path_search
            .text()
            .await
            .unwrap()
            .contains("项目/Alpha/00-总览/INDEX.md")
    );
}
