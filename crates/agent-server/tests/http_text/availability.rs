use super::support::{denied, fixture};
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

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

/// 身份库读不出来（BUG-030）：令牌真假与类型是否吊销都未知，回 503、不带 Bearer challenge，
/// 不交付内容也不碰会话；恢复后同一会话照常续读。注入方式同 MCP 契约：私有库文件改成 0640。
/// 三次请求都在认证门被挡下；`dispatch` 交付前的最后复核要两次读之间身份库恰好坏掉才走得到，
/// 不加生产钩子没法确定地测，靠代码复查。
#[tokio::test]
async fn an_unreadable_identity_store_answers_503_without_a_challenge_and_keeps_the_session() {
    let (_temp, f) = fixture().await;
    let (session, _, _) = f.handshake().await;
    let database = f.data.join("agent.sqlite3");
    let note = [("path", "项目/Alpha/00-总览/INDEX.md")];
    fs::set_permissions(&database, fs::Permissions::from_mode(0o640)).unwrap();
    for request in [
        f.get("/read/note", Some(&session)).query(&note),
        f.get("/read/handshake", Some(&session)),
        f.get("/read/handshake", None),
    ] {
        let response = request.send().await.unwrap();
        assert!(response.headers().get("www-authenticate").is_none());
        denied(response, 503).await;
    }
    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    let recovered = f
        .get("/read/note", Some(&session))
        .query(&note)
        .send()
        .await
        .unwrap();
    assert_eq!(recovered.status(), 200);
    assert!(recovered.text().await.unwrap().contains("Alpha · 总览"));
    let reasons = f
        .ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|event| {
            event.event().payload().as_value()["reason"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(reasons, ["identity_unavailable"; 3]);
}
