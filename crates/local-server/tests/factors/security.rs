//! 通过真实Unix连接覆盖浏览器门禁；TCP或oneshot缺少内核来源证明时不能提权。

use super::{client::Client, fixture::Fixture, support::*};
use axum::body::Body;
use tower::ServiceExt;

#[tokio::test]
async fn factor_routes_require_all_browser_gates_and_only_stale_session_clears_cookie() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    for (name, status) in [
        ("cf-access-jwt-assertion", 401),
        ("origin", 403),
        ("x-csrf-token", 403),
        ("x-studiolo-request", 403),
        ("sec-fetch-site", 403),
        ("sec-fetch-mode", 403),
        ("sec-fetch-dest", 403),
        ("cf-connecting-ip", 403),
    ] {
        let mut request = f.request("POST", STEP_UP, Some(&login), &body);
        request.headers_mut().remove(name);
        f.send(request, status).await.kept_cookie();
    }
    for (name, value) in [
        ("origin", "https://other.example.com"),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
        ("sec-fetch-user", "?1"),
        ("x-csrf-token", "not-a-csrf"),
    ] {
        let mut request = f.request("POST", STEP_UP, Some(&login), &body);
        request.headers_mut().insert(name, value.parse().unwrap());
        f.send(request, 403).await.kept_cookie();
    }
    f.send(f.request("POST", STEP_UP, None, &body), 401)
        .await
        .cleared_cookie();
    f.elevate(&login, false).await;
    f.send(
        f.request("POST", REVOKE, Some(&login), "{\"scope\":\"current\"}"),
        200,
    )
    .await
    .cleared_cookie();
    f.send(f.request("POST", STEP_UP, Some(&login), &body), 401)
        .await
        .cleared_cookie();
    f.stop().await;
}

#[tokio::test]
async fn source_is_revalidated_on_every_request_of_one_unix_connection() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let mut client = Client::connect(&f.socket).await;
    let bad = format!("{{\"code\":\"{}\"}}", wrong(&OLD, f.now()));
    assert_eq!(
        client
            .send(f.request("POST", STEP_UP, Some(&login), &bad))
            .await
            .status,
        403
    );
    let good = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    let mut missing = f.request("POST", STEP_UP, Some(&login), &good);
    missing.headers_mut().remove("cf-connecting-ip");
    let rejected = client.send(missing).await;
    assert_eq!(rejected.status, 403);
    rejected.kept_cookie();
    let mut changed = f.request("POST", STEP_UP, Some(&login), &good);
    changed
        .headers_mut()
        .insert("cf-connecting-ip", "::ffff:192.0.2.7".parse().unwrap());
    assert_eq!(client.send(changed).await.status, 200);
    drop(client);
    assert_eq!(f.events("totp.step_up").len(), 2);
    f.stop().await;
}

#[tokio::test]
async fn forged_proxy_headers_without_unix_proof_cannot_activate_factors() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    let (parts, bytes) = f.request("POST", STEP_UP, Some(&login), &body).into_parts();
    let response = f
        .router()
        .await
        .oneshot(axum::http::Request::from_parts(parts, Body::from(bytes)))
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert!(!response.headers().contains_key("set-cookie"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.router().await;
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let request = f.request("POST", STEP_UP, Some(&login), &body);
    let reply = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!("http://{address}{STEP_UP}"))
        .headers(request.headers().clone())
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 403);
    assert!(!reply.headers().contains_key("set-cookie"));
    server.abort();
    let _ = server.await;
    f.elevate(&login, false).await;
    f.stop().await;
}

#[tokio::test]
async fn forwarded_ip_syntax_and_duplicate_headers_never_replace_valid_source_proof() {
    let mut f = Fixture::new(true).await;
    let login = f.issue().await;
    let body = format!("{{\"code\":\"{}\"}}", otp(&OLD, f.now()));
    for value in [
        "",
        "192.0.2.1,192.0.2.2",
        "192.0.2.1:9",
        "[2001:db8::1]",
        "fe80::1%en0",
        "not-an-ip",
    ] {
        let mut request = f.request("POST", STEP_UP, Some(&login), &body);
        request
            .headers_mut()
            .insert("cf-connecting-ip", value.parse().unwrap());
        request
            .headers_mut()
            .insert("x-forwarded-for", "192.0.2.1".parse().unwrap());
        f.send(request, 403).await.kept_cookie();
    }
    for name in ["cf-connecting-ip", "origin", "x-csrf-token"] {
        let mut request = f.request("POST", STEP_UP, Some(&login), &body);
        let value = request.headers()[name].clone();
        request.headers_mut().append(name, value);
        f.send(request, 403).await.kept_cookie();
    }
    f.elevate(&login, false).await;
    f.stop().await;
}
