//! 实际正文和闭合 JSON 契约；格式错误不能进入 Auth 业务写入。

use super::support::*;
use axum::http::HeaderValue;

#[tokio::test]
async fn aggregate_header_budget_rejects_oversized_and_wide_requests_before_issuance() {
    let f = Fixture::new(true).await;
    let mut oversized = f.request("POST", SESSIONS, None, "{}");
    oversized
        .headers_mut()
        .insert("x-fixture-padding", "a".repeat(32 * 1024).parse().unwrap());
    f.send(oversized, 431).await;
    let mut wide = f.request("POST", SESSIONS, None, "{}");
    for index in 0..129 {
        let name =
            axum::http::HeaderName::from_bytes(format!("x-fixture-{index}").as_bytes()).unwrap();
        wide.headers_mut()
            .insert(name, HeaderValue::from_static("x"));
    }
    f.send(wide, 431).await;
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn issuance_accepts_only_an_empty_json_object_with_bounded_actual_bytes() {
    let f = Fixture::new(true).await;
    for body in [
        "",
        "null",
        "[]",
        "true",
        "{}{}",
        "{",
        "{\"x\":1}",
        "{\"scope\":\"all\"}",
        "{\"x\":1,\"x\":2}",
    ] {
        f.send(f.request("POST", SESSIONS, None, body), 400).await;
    }
    // 不设置 Content-Length；必须约束真正读到的字节，而非相信请求声明。
    let body = format!("{{}}{}", " ".repeat(1023));
    f.send(f.request("POST", SESSIONS, None, &body), 413).await;
    assert!(f.events("session.issue").is_empty());
    let body = format!("{{}}{}", " ".repeat(1022));
    f.send(f.request("POST", SESSIONS, None, &body), 201).await;
    assert_eq!(f.events("session.issue").len(), 2);
}

#[tokio::test]
async fn absent_wrong_and_duplicate_content_type_are_rejected_without_issuance() {
    let f = Fixture::new(true).await;
    for value in [
        None,
        Some("text/plain"),
        Some("application/x-www-form-urlencoded"),
        Some("application/jsonp"),
    ] {
        let mut request = f.request("POST", SESSIONS, None, "{}");
        request.headers_mut().remove("content-type");
        if let Some(value) = value {
            request
                .headers_mut()
                .insert("content-type", value.parse().unwrap());
        }
        f.send(request, 415).await;
    }
    let mut duplicate = f.request("POST", SESSIONS, None, "{}");
    duplicate
        .headers_mut()
        .append("content-type", HeaderValue::from_static("application/json"));
    f.send(duplicate, 403).await;
    assert!(f.events("session.issue").is_empty());
}

#[tokio::test]
async fn revoke_scope_fields_and_repeated_keys_are_closed_before_any_revocation() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    for body in [
        "{}",
        "null",
        "[]",
        "{\"scope\":\"other\"}",
        "{\"scope\":\"session\"}",
        "{\"scope\":\"session\",\"session_id\":\"invalid\"}",
        "{\"scope\":\"current\",\"session_id\":\"00000000000000000000000000000000\"}",
        "{\"scope\":\"all\",\"session_id\":null}",
        "{\"scope\":\"current\",\"x\":0}",
        "{\"scope\":\"current\",\"scope\":\"all\"}",
        "{\"scope\":\"session\",\"session_id\":\"bad\",\"session_id\":\"bad\"}",
    ] {
        let mut request = f.request("POST", REVOKE, Some(&issued.cookie), body);
        request
            .headers_mut()
            .insert("x-csrf-token", issued.csrf.parse().unwrap());
        f.send(request, 400).await;
    }
    assert!(f.events("session.revoked").is_empty());
    f.send(f.request("GET", OWN, Some(&issued.cookie), ""), 200)
        .await;
}

#[tokio::test]
async fn cookie_header_size_and_pair_count_are_bounded_without_secret_echo() {
    let f = Fixture::new(true).await;
    let issued = f.issue().await;
    for cookie in [
        format!("other={}; {}", "a".repeat(8192), issued.cookie),
        format!(
            "{}; {}",
            (0..64)
                .map(|i| format!("c{i}=x"))
                .collect::<Vec<_>>()
                .join("; "),
            issued.cookie
        ),
    ] {
        let response = f.send(f.request("GET", OWN, Some(&cookie), ""), 400).await;
        let body = json(response).await.to_string();
        assert!(!body.contains(&issued.token));
        assert!(!body.contains(&issued.csrf));
    }
    let cookie = format!(
        "{}; {}",
        (0..63)
            .map(|i| format!("c{i}=x"))
            .collect::<Vec<_>>()
            .join("; "),
        issued.cookie
    );
    f.send(f.request("GET", OWN, Some(&cookie), ""), 200).await;
}

#[tokio::test]
async fn misleading_lengths_encoding_and_nonempty_get_body_are_rejected() {
    let f = Fixture::new(true).await;
    for (name, value, status) in [
        ("content-length", "1", 400),
        ("content-length", "3", 400),
        ("content-length", "-1", 400),
        ("content-encoding", "gzip", 415),
        ("transfer-encoding", "identity", 400),
    ] {
        let mut request = f.request("POST", SESSIONS, None, "{}");
        request.headers_mut().insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        f.send(request, status).await;
    }
    let mut ambiguous = f.request("POST", SESSIONS, None, "{}");
    ambiguous
        .headers_mut()
        .insert("content-length", "2".parse().unwrap());
    ambiguous
        .headers_mut()
        .insert("transfer-encoding", "chunked".parse().unwrap());
    f.send(ambiguous, 400).await;
    assert!(f.events("session.issue").is_empty());
    let issued = f.issue().await;
    f.send(f.request("GET", OWN, Some(&issued.cookie), "{}"), 413)
        .await;
}
