//! 因子JSON与传输预算的负向检查必须发生在任何OTP消费或秘密交付之前。

use super::{fixture::Fixture, support::*};

#[tokio::test]
async fn factor_bodies_reject_unknown_duplicate_wrong_type_and_non_object_json() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let valid = otp(&OLD, f.now());
    for body in [
        "".to_owned(),
        "{}".into(),
        "[]".into(),
        "null".into(),
        "{\"code\":123456}".into(),
        "{\"code\":\"12345\"}".into(),
        "{\"code\":\"１２３４５６\"}".into(),
        format!("{{\"code\":\"{valid}\",\"code\":\"{valid}\"}}"),
        format!("{{\"code\":\"{valid}\",\"seed\":\"forbidden\"}}"),
        format!("{{\"code\":\"{valid}\"}} trailing"),
    ] {
        f.send(f.request("POST", STEP_UP, Some(&login), &body), 400)
            .await
            .kept_cookie();
    }
    for body in ["[]", "{\"count\":10}", "{\"code\":\"123456\"}"] {
        f.send(f.request("POST", CODES, Some(&login), body), 400)
            .await
            .kept_cookie();
    }
    let code = "r1.0000000000000001.00.00000000000000000000000000000000";
    for body in [
        "{}".to_owned(),
        "[]".into(),
        "{\"recovery_code\":null,\"code\":\"123456\"}".into(),
        format!("{{\"recovery_code\":\"{code}\",\"code\":\"123456\",\"code\":\"123456\"}}"),
        format!("{{\"recovery_code\":\"{code}\",\"code\":\"123456\",\"seed_ref\":\"unknown\"}}"),
        format!(
            "{{\"recovery_code\":\"{code}\",\"recovery_code\":\"{code}\",\"code\":\"123456\"}}"
        ),
    ] {
        f.send(f.request("POST", RECOVER, Some(&login), &body), 400)
            .await
            .kept_cookie();
    }
    assert!(f.events("totp.step_up").is_empty());
    assert!(f.events("totp.recovered").is_empty());
    f.elevate(&login, false).await;
    f.stop().await;
}

#[tokio::test]
async fn all_factor_methods_and_queries_are_closed() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    for path in [STEP_UP, CODES, RECOVER] {
        for method in ["GET", "HEAD", "OPTIONS", "PUT", "DELETE"] {
            let reply = f.send(f.request(method, path, Some(&login), ""), 405).await;
            assert_eq!(reply.headers["allow"], "POST");
            reply.kept_cookie();
            if method == "HEAD" {
                assert!(reply.body.is_empty());
            }
        }
        f.send(
            f.request("POST", &format!("{path}?"), Some(&login), "{}"),
            400,
        )
        .await
        .kept_cookie();
        f.send(
            f.request("POST", &format!("{path}?code=123456"), Some(&login), "{}"),
            400,
        )
        .await
        .kept_cookie();
    }
    f.stop().await;
}

#[tokio::test]
async fn factor_media_and_actual_body_size_limits_are_checked_before_auth_work() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    for (name, value) in [
        ("content-type", "text/plain"),
        ("content-type", "application/json; charset=utf-8"),
        ("content-encoding", "gzip"),
    ] {
        let mut request = f.request("POST", STEP_UP, Some(&login), &body);
        request.headers_mut().insert(name, value.parse().unwrap());
        f.send(request, 415).await.kept_cookie();
    }
    let oversized = format!("{body}{}", " ".repeat(1025));
    f.send(f.request("POST", STEP_UP, Some(&login), &oversized), 413)
        .await
        .kept_cookie();
    let mut chunked = f.request(
        "POST",
        STEP_UP,
        Some(&login),
        &format!("{:x}\r\n{oversized}\r\n0\r\n\r\n", oversized.len()),
    );
    chunked
        .headers_mut()
        .insert("transfer-encoding", "chunked".parse().unwrap());
    f.send(chunked, 413).await.kept_cookie();
    let mut slow = f.request("POST", STEP_UP, Some(&login), "{");
    slow.headers_mut()
        .insert("content-length", "2".parse().unwrap());
    f.send(slow, 408).await.kept_cookie();
    assert!(f.events("totp.step_up").is_empty());
    f.elevate(&login, false).await;
    f.stop().await;
}
