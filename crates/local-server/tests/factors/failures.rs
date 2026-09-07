//! 错误因子、限流和真实审计故障维持原状态机语义，不误清仍可能有效的Cookie。

use super::{fixture::Fixture, support::*};
use studiolo_auth::Totp;

#[tokio::test]
async fn wrong_otp_and_expired_l3_are_forbidden_without_logging_out() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let code = wrong(&OLD, f.now());
    let body = format!("{{\"code\":\"{code}\"}}");
    let failed = f
        .send(f.request("POST", STEP_UP, Some(&login), &body), 403)
        .await;
    failed.kept_cookie();
    assert!(!String::from_utf8(failed.body).unwrap().contains(&code));
    f.send(f.request("POST", CODES, Some(&login), "{}"), 403)
        .await
        .kept_cookie();
    f.elevate(&login, false).await;
    f.advance(300_000);
    f.send(f.request("POST", CODES, Some(&login), "{}"), 403)
        .await
        .kept_cookie();
    f.send(f.request("GET", OWN, Some(&login), ""), 200).await;
    f.stop().await;
}

#[tokio::test]
async fn account_rate_limit_survives_other_cookie_address_and_reopening() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let body = format!("{{\"code\":\"{}\"}}", wrong(&OLD, f.now()));
    // 本地登记已用掉账号的一次预算；四次HTTP失败后仍不能借新来源绕过账号限制。
    for index in 0..5 {
        let mut request = f.request("POST", STEP_UP, Some(&login), &body);
        request.headers_mut().insert(
            "cf-connecting-ip",
            format!("192.0.2.{}", index + 1).parse().unwrap(),
        );
        f.send(request, if index < 4 { 403 } else { 429 })
            .await
            .kept_cookie();
    }
    f.restart().await;
    let fresh = f.issue().await;
    let mut request = f.request("POST", STEP_UP, Some(&fresh), &body);
    request
        .headers_mut()
        .insert("cf-connecting-ip", "2001:db8::7".parse().unwrap());
    f.send(request, 429).await.kept_cookie();
    f.stop().await;
}

#[tokio::test]
async fn uncertain_audit_failure_preserves_cookie_and_flush_does_not_grant_l3() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let audit = f.root.join("audit/audit.sqlite3");
    mode(&audit, 0o400);
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    let result = f
        .send(f.request("POST", STEP_UP, Some(&login), &body), 503)
        .await;
    mode(&audit, 0o600);
    result.kept_cookie();
    assert!(
        result.json()["error"]
            .as_str()
            .unwrap()
            .contains("可能已生效")
    );
    f.send(f.request("POST", STEP_UP, Some(&login), &body), 409)
        .await
        .kept_cookie();
    f.send(f.request("GET", OWN, Some(&login), ""), 200).await;
    let (totp, _) = Totp::attach(f.manager(), seed(false)).unwrap();
    totp.flush_audit().unwrap();
    f.advance(300_000);
    totp.flush_audit().unwrap();
    drop(totp);
    f.send(f.request("POST", CODES, Some(&login), "{}"), 403)
        .await
        .kept_cookie();
    f.stop().await;
}
