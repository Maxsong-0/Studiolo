use super::{common::*, fixture::Fixture};

#[tokio::test]
async fn paired_session_and_browser_gates_remain_required_without_action_headers() {
    let mut fixture = Fixture::new(true).await;
    let login = fixture.issue().await;
    state(&fixture, &login).await;
    for (name, status) in [
        ("cf-access-jwt-assertion", 401),
        ("x-studiolo-request", 403),
        ("sec-fetch-site", 403),
        ("sec-fetch-mode", 403),
        ("sec-fetch-dest", 403),
    ] {
        let mut value = request(&fixture, Some(&login));
        value.headers_mut().remove(name);
        fixture.send(value, status).await.kept_cookie();
    }
    for (name, value) in [
        ("host", "other.invalid"),
        ("origin", "https://other.invalid"),
        ("sec-fetch-site", "same-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
        ("sec-fetch-user", "?1"),
    ] {
        let mut input = request(&fixture, Some(&login));
        input.headers_mut().insert(name, value.parse().unwrap());
        fixture.send(input, 403).await.kept_cookie();
    }
    let mut duplicate = request(&fixture, Some(&login));
    duplicate
        .headers_mut()
        .append("x-studiolo-request", "1".parse().unwrap());
    fixture.send(duplicate, 403).await.kept_cookie();
    fixture
        .send(request(&fixture, None), 401)
        .await
        .cleared_cookie();
    let other = fixture.issue().await;
    let (prefix, _) = login.cookie.split_once('~').unwrap();
    let mut mixed = request(&fixture, Some(&login));
    mixed.headers_mut().insert(
        "cookie",
        format!("{prefix}~{}", other.csrf).parse().unwrap(),
    );
    fixture.send(mixed, 401).await.cleared_cookie();
    state(&fixture, &login).await;
    fixture.stop().await;
}

#[tokio::test]
async fn only_get_without_query_or_body_is_accepted() {
    let mut fixture = Fixture::new(true).await;
    let login = fixture.issue().await;
    for method in ["HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"] {
        let reply = fixture
            .send(fixture.request(method, STATE, Some(&login), ""), 405)
            .await;
        assert_eq!(reply.headers["allow"], "GET");
        reply.kept_cookie();
        if method == "HEAD" {
            assert!(reply.body.is_empty());
        }
    }
    for suffix in [
        "?",
        "?unknown=state-private-canary",
        "?code=state-private-canary",
    ] {
        let reply = fixture
            .send(
                fixture.request("GET", &format!("{STATE}{suffix}"), Some(&login), ""),
                400,
            )
            .await;
        reply.kept_cookie();
        no_secrets(&reply, &login, &["state-private-canary"]);
    }
    for body in [" ", "{}", "state-private-canary"] {
        let reply = fixture
            .send(fixture.request("GET", STATE, Some(&login), body), 413)
            .await;
        reply.kept_cookie();
        no_secrets(&reply, &login, &["state-private-canary"]);
    }
    let mut chunked = fixture.request("GET", STATE, Some(&login), "1\r\nx\r\n0\r\n\r\n");
    chunked
        .headers_mut()
        .insert("transfer-encoding", "chunked".parse().unwrap());
    fixture.send(chunked, 413).await.kept_cookie();
    let mut encoded = request(&fixture, Some(&login));
    encoded
        .headers_mut()
        .insert("content-encoding", "gzip".parse().unwrap());
    fixture.send(encoded, 415).await.kept_cookie();
    state(&fixture, &login).await;
    fixture.stop().await;
}
