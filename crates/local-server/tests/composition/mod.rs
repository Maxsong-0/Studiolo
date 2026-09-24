//! 生产装配顺序的私有Seam测试；固定公钥Adapter不进入production构建。

mod canonical;
mod factors;
mod fixture;
mod repository;
mod repository_approved;
mod residency;
mod sync_plane;
#[path = "../access/test_support.rs"]
mod test_support;
mod writer;
mod writer_m0;
mod writer_transport;

use super::{StartupError, prepare_with};
use fixture::Fixture;
use std::fs;

#[tokio::test]
async fn ready_configuration_builds_session_routes_before_loopback_listen() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let f = Fixture::new(true);
    let prepared = prepare_with(f.options(), |_| async {
        f.unbound();
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    f.unbound();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/sessions")
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
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let response = prepared.app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), 201);
    assert!(response.headers().contains_key("set-cookie"));
    let (listener, _) = prepared.bind().await.unwrap();
    assert_eq!(
        listener.local_addr().unwrap().ip(),
        std::net::Ipv4Addr::LOCALHOST
    );
    assert_eq!(listener.local_addr().unwrap().port(), f.port);
}

#[tokio::test]
async fn unregistered_store_fails_before_external_keys_and_listen() {
    let f = Fixture::new(false);
    let result = prepare_with(f.options(), |_| async { panic!("未登记不得取钥") }).await;
    assert!(matches!(
        result,
        Err(StartupError::Auth(
            studiolo_auth::SessionError::FactorUnavailable
        ))
    ));
    f.unbound();
}

#[tokio::test]
async fn missing_mismatched_or_wrong_existing_stores_fail_before_external_keys() {
    use std::os::unix::fs::PermissionsExt;
    use studiolo_auth::SessionError;
    for case in [
        "missing-auth",
        "missing-audit",
        "wrong-audit",
        "owner",
        "ttl",
    ] {
        let f = Fixture::new(true);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
        match case {
            "missing-auth" => fs::remove_file(f.root.join("auth/auth.sqlite3")).unwrap(),
            "missing-audit" => fs::remove_file(f.root.join("audit/audit.sqlite3")).unwrap(),
            "wrong-audit" => {
                let path = f.root.join("wrong-audit");
                fs::create_dir(&path).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                drop(studiolo_audit_ledger::SqliteAuditLedger::open(&path).unwrap());
                value["audit_directory"] = serde_json::json!(path);
            }
            "owner" => value["owner"]["subject"] = "another-fictional-person".into(),
            "ttl" => value["idle_seconds"] = 900.into(),
            _ => unreachable!(),
        }
        fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
        let result = prepare_with(f.options(), |_| async {
            panic!("无效双库不得取钥")
        })
        .await;
        match case {
            "missing-auth" => assert!(matches!(
                result,
                Err(StartupError::Auth(SessionError::MissingStore))
            )),
            "missing-audit" => assert!(matches!(result, Err(StartupError::Audit))),
            "wrong-audit" => assert!(matches!(
                result,
                Err(StartupError::Auth(SessionError::AuditUnavailable))
            )),
            _ => assert!(matches!(
                result,
                Err(StartupError::Auth(SessionError::Configuration))
            )),
        }
        f.unbound();
    }
}

#[tokio::test]
async fn key_initialization_failure_does_not_return_a_local_router_or_listener() {
    let f = Fixture::new(true);
    let result = prepare_with(f.options(), |_| async {
        f.unbound();
        Err(studiolo_auth::AccessError::Configuration)
    })
    .await;
    assert!(matches!(result, Err(StartupError::Access)));
    f.unbound();
}

#[tokio::test]
async fn root_replaced_during_external_initialization_is_rejected() {
    let f = Fixture::new(true);
    let result = prepare_with(f.options(), |_| async {
        fs::rename(f.root.join("vault"), f.root.join("retained-vault")).unwrap();
        fs::create_dir(f.root.join("vault")).unwrap();
        Ok(test_support::verifier().await)
    })
    .await;
    assert!(matches!(result, Err(StartupError::Configuration)));
    f.unbound();
}

#[tokio::test]
async fn prepared_configuration_is_rechecked_immediately_before_binding() {
    let f = Fixture::new(true);
    let prepared = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    fs::rename(&f.config, f.root.join("config/retained.json")).unwrap();
    fs::copy(f.root.join("config/retained.json"), &f.config).unwrap();
    assert!(matches!(
        prepared.bind().await,
        Err(StartupError::Configuration)
    ));
    f.unbound();
}

#[tokio::test]
async fn incomplete_vault_scan_fails_before_any_listener() {
    let f = Fixture::new(true);
    std::os::unix::fs::symlink("note.md", f.root.join("vault/link.md")).unwrap();
    let result = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await;
    assert!(matches!(
        result,
        Err(StartupError::Router {
            reason: "副本扫描不完整"
        })
    ));
    f.unbound();
}
