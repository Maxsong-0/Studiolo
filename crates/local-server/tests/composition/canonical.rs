//! canonical启动不初始化缺失标记，并在真实会话门禁后提供单页读取。

use super::{Fixture, StartupError, prepare_with, test_support};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use std::fs;
use studiolo_vault_broker::FsVaultBroker;
use tower::ServiceExt;

#[tokio::test]
async fn canonical_identity_is_rechecked_immediately_before_binding() {
    for mutation in ["remove", "corrupt", "replace"] {
        let f = Fixture::new(true);
        drop(FsVaultBroker::open(&f.root.join("vault"), &f.root.join("data")).unwrap());
        let mut options = f.options();
        options.canonical = true;
        let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
            .await
            .unwrap();
        let marker = f.root.join("vault/.workbench/vault.json");
        match mutation {
            "remove" => fs::remove_file(marker).unwrap(),
            "corrupt" => fs::write(marker, b"not-json").unwrap(),
            "replace" => {
                drop(
                    FsVaultBroker::open(&f.root.join("other"), &f.root.join("other-data")).unwrap(),
                );
                fs::copy(f.root.join("other/.workbench/vault.json"), marker).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            prepared.bind().await,
            Err(StartupError::ReadRoots)
        ));
        f.unbound();
    }
}

#[tokio::test]
async fn canonical_missing_or_bad_identity_fails_before_keys_and_never_initializes() {
    for bad in [false, true] {
        let f = Fixture::new(true);
        if bad {
            fs::create_dir(f.root.join("vault/.workbench")).unwrap();
            fs::write(f.root.join("vault/.workbench/vault.json"), b"not-json").unwrap();
        }
        let mut options = f.options();
        options.canonical = true;
        let result = prepare_with(options, |_| async { panic!("无效canonical不得取钥") }).await;
        assert!(matches!(result, Err(StartupError::ReadRoots)));
        assert_eq!(f.root.join("vault/.workbench").exists(), bad);
        f.unbound();
    }
}

#[tokio::test]
async fn explicit_canonical_startup_serves_authoritative_file_after_session_login() {
    let f = Fixture::new(true);
    drop(FsVaultBroker::open(&f.root.join("vault"), &f.root.join("data")).unwrap());
    let marker = fs::read(f.root.join("vault/.workbench/vault.json")).unwrap();
    let mut options = f.options();
    options.canonical = true;
    let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
        .await
        .unwrap();
    f.unbound();
    let request = |method: &str, path: &str, cookie: Option<&str>| {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", format!("127.0.0.1:{}", f.port))
            .header("origin", "https://studiolo.example.com")
            .header(
                "cf-access-jwt-assertion",
                test_support::signed(&test_support::claims()),
            )
            .header("x-studiolo-request", "1")
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-dest", "empty")
            .header("content-type", "application/json");
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        builder
            .body(if method == "POST" {
                Body::from("{}")
            } else {
                Body::empty()
            })
            .unwrap()
    };
    let anonymous = prepared
        .app
        .clone()
        .oneshot(request("GET", "/api/v1/file?path=note.md", None))
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    let login = prepared
        .app
        .clone()
        .oneshot(request("POST", "/api/v1/sessions", None))
        .await
        .unwrap();
    assert_eq!(login.status(), 201);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let file = prepared
        .app
        .clone()
        .oneshot(request("GET", "/api/v1/file?path=note.md", Some(cookie)))
        .await
        .unwrap();
    assert_eq!(file.status(), 200);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(file.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(body["content"], "# synthetic note\n");
    assert_eq!(body["revision"].as_str().unwrap().len(), 64);
    assert_eq!(
        fs::read(f.root.join("vault/.workbench/vault.json")).unwrap(),
        marker
    );
    let (listener, _) = prepared.bind().await.unwrap();
    assert_eq!(listener.local_addr().unwrap().port(), f.port);
}
