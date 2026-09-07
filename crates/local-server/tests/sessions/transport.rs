//! 真实回环 HTTP；人工传 Cookie，不声称覆盖 HTTPS 浏览器的 Cookie 行为。

use super::{
    support::*,
    test_support::{claims, signed},
};
use std::{
    io::{Read, Write},
    time::Duration,
};

#[tokio::test]
async fn real_loopback_transport_enforces_body_limits_and_carries_the_issued_pair() {
    let f = Fixture::new(true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = f.app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let endpoint = format!("http://{address}{SESSIONS}");
    let headers = || {
        let mut headers = reqwest::header::HeaderMap::new();
        for (name, value) in [
            ("host", "127.0.0.1:4319"),
            ("origin", ORIGIN),
            ("x-studiolo-request", "1"),
            ("sec-fetch-site", "same-origin"),
            ("sec-fetch-mode", "cors"),
            ("sec-fetch-dest", "empty"),
            ("content-type", "application/json"),
        ] {
            headers.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers.insert(
            "cf-access-jwt-assertion",
            signed(&claims()).parse().unwrap(),
        );
        headers
    };
    let too_large = client
        .post(&endpoint)
        .headers(headers())
        .body(format!("{{}}{}", " ".repeat(1023)))
        .send()
        .await
        .unwrap();
    assert_eq!(too_large.status().as_u16(), 413);
    assert!(!too_large.headers().contains_key("set-cookie"));
    assert_eq!(too_large.headers()["cache-control"], "no-store");
    assert!(f.events("session.issue").is_empty());

    // 正文只送一半并保持连接；读取必须在固定两秒预算后失败，而非进入签发。
    let token = signed(&claims());
    let slow = tokio::task::spawn_blocking(move || {
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let head = format!("POST {SESSIONS} HTTP/1.1\r\nHost: 127.0.0.1:4319\r\nOrigin: {ORIGIN}\r\nCf-Access-Jwt-Assertion: {token}\r\nX-Studiolo-Request: 1\r\nSec-Fetch-Site: same-origin\r\nSec-Fetch-Mode: cors\r\nSec-Fetch-Dest: empty\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{");
        stream.write_all(head.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }).await.unwrap();
    assert!(slow.starts_with("HTTP/1.1 408 "));
    assert!(!slow.to_ascii_lowercase().contains("set-cookie:"));
    assert!(f.events("session.issue").is_empty());

    let response = client
        .post(&endpoint)
        .headers(headers())
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 201);
    let raw = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .to_owned();
    cookie_attributes(&raw);
    let cookie = raw.split(';').next().unwrap();
    let body: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    let own = client
        .get(format!("http://{address}{OWN}"))
        .headers(headers())
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(own.status().as_u16(), 200);
    let own: serde_json::Value = serde_json::from_slice(&own.bytes().await.unwrap()).unwrap();
    assert_eq!(own["session"]["id"], body["session"]["id"]);
    assert_eq!(own["csrf_token"], body["csrf_token"]);
    assert_eq!(f.events("session.issue").len(), 2);
    server.abort();
    let _ = server.await;
}
