use super::support::{denied, fixture};
use reqwest::Method;
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

const ROUTES: [&str; 4] = [
    "/read/handshake",
    "/read/entries?project=School",
    "/read/note?path=School%2Fsecret.md",
    "/read/search?project=School&text=secret",
];

#[tokio::test]
async fn authentication_is_required_for_every_route_and_rechecked_after_revocation() {
    let (_temp, f) = fixture().await;
    let (session, _, _) = f.handshake().await;
    for token in [None, Some(""), Some("sag1_wrong")] {
        for route in ROUTES {
            denied(
                f.request(Method::GET, route, token, Some(&session))
                    .send()
                    .await
                    .unwrap(),
                401,
            )
            .await;
        }
    }
    assert!(
        f.store()
            .revoke_type("claude", "2026-09-10T01:00:00Z")
            .unwrap()
    );
    for route in ROUTES {
        denied(f.get(route, Some(&session)).send().await.unwrap(), 401).await;
    }
    denied(f.get("/read/handshake", None).send().await.unwrap(), 401).await;
    let denied_events = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap();
    assert_eq!(denied_events.len(), 17);
    assert!(
        denied_events
            .iter()
            .all(|record| record.event().payload().as_value()["reason"] == "unauthenticated")
    );
}

#[tokio::test]
async fn content_requires_a_session_and_instance_ids_never_grant_access() {
    let (_temp, f) = fixture().await;
    let (_, instance, _) = f.handshake().await;
    for session in [
        None,
        Some("00000000000000000000000000"),
        Some(instance.as_str()),
    ] {
        for route in &ROUTES[1..] {
            denied(f.get(route, session).send().await.unwrap(), 401).await;
        }
    }
    denied(
        f.get("/read/handshake", Some("00000000000000000000000000"))
            .send()
            .await
            .unwrap(),
        401,
    )
    .await;
}

#[tokio::test]
async fn another_authenticated_type_cannot_use_a_session_or_its_handshake() {
    let (_temp, f) = fixture().await;
    let (session, _, _) = f.handshake().await;
    let other = f
        .store()
        .issue_type("codex", "2026-09-10T00:00:01Z")
        .unwrap()
        .token;
    for route in ROUTES {
        denied(
            f.request(Method::GET, route, Some(&other), Some(&session))
                .send()
                .await
                .unwrap(),
            401,
        )
        .await;
    }
    let own = f
        .request(Method::GET, "/read/handshake", Some(&other), None)
        .send()
        .await
        .unwrap();
    assert_eq!(own.status(), 200);
    let own_session = super::support::header(&own, "studiolo-session-id");
    assert!(own.text().await.unwrap().contains("类型 codex"));
    f.store()
        .revoke_type("claude", "2026-09-10T01:00:00Z")
        .unwrap();
    let own = f
        .request(Method::GET, "/read/note", Some(&other), Some(&own_session))
        .query(&[("path", "项目/Alpha/00-总览/INDEX.md")])
        .send()
        .await
        .unwrap();
    assert_eq!(own.status(), 200, "revoking claude must not revoke codex");
    super::support::assert_headers(&own, true);
}
