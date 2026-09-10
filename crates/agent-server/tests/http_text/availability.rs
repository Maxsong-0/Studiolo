use super::support::{denied, fixture};
use std::{fs, os::unix::fs::PermissionsExt};

#[tokio::test]
async fn audit_failure_withholds_content_and_the_handshake_until_recovery() {
    let (_temp, f) = fixture().await;
    let (session, _, _) = f.handshake().await;
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o750)).unwrap();
    for session in [None, Some(session.as_str())] {
        let body = denied(f.get("/read/handshake", session).send().await.unwrap(), 503).await;
        assert!(
            body.contains("审计不可用") && body.contains("outage"),
            "{body}"
        );
        assert!(!body.contains("# Studiolo 路牌") && !body.contains("/read/"));
    }
    for (route, params) in [
        ("/read/note", vec![("path", "项目/Alpha/00-总览/INDEX.md")]),
        ("/read/entries", vec![("project", "项目/Alpha")]),
        (
            "/read/search",
            vec![("project", "项目/Alpha"), ("text", "Alpha 的事")],
        ),
    ] {
        denied(
            f.get(route, Some(&session))
                .query(&params)
                .send()
                .await
                .unwrap(),
            503,
        )
        .await;
    }
    assert!(f.state.degraded().is_some());
    fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(0o700)).unwrap();
    let (_, _, recovered) = f.handshake().await;
    assert!(recovered.contains("# Studiolo 路牌") && recovered.contains("Alpha"));
    assert!(f.state.degraded().is_none());
    let outages = f.store().outages().unwrap();
    assert_eq!(outages.len(), 1);
    assert!(outages[0].2.is_some(), "recovery closes the outage");
    assert!(
        outages[0].3 >= 5,
        "every withheld operation counts as denied"
    );
}
