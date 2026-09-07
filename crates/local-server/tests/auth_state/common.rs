use super::{client::Reply, fixture::Fixture, support::Login};
use axum::http::Request;
use serde_json::Value;

pub const STATE: &str = "/api/v1/auth/state";

pub fn request(fixture: &Fixture, login: Option<&Login>) -> Request<Vec<u8>> {
    let mut request = fixture.request("GET", STATE, login, "");
    // GET 仍校验 Cookie 双秘密配对，但不依赖因子动作的来源 IP 或独立 CSRF 头。
    for name in ["cf-connecting-ip", "x-csrf-token", "origin", "content-type"] {
        request.headers_mut().remove(name);
    }
    request
}

pub async fn state(fixture: &Fixture, login: &Login) -> Value {
    let reply = fixture.send(request(fixture, Some(login)), 200).await;
    reply.kept_cookie();
    no_secrets(&reply, login, &[]);
    let value = reply.json();
    keys(&value, &["factors"]);
    let factors = &value["factors"];
    keys(
        factors,
        &[
            "observed_at_ms",
            "recent_authentication",
            "recovery_candidate_available",
            "pending",
        ],
    );
    assert_eq!(factors["observed_at_ms"], fixture.now());
    assert!(factors["recovery_candidate_available"].is_boolean());
    assert!(factors["pending"].is_boolean());
    if !factors["recent_authentication"].is_null() {
        keys(
            &factors["recent_authentication"],
            &["verified_at_ms", "expires_at_ms"],
        );
        assert!(factors["recent_authentication"]["verified_at_ms"].is_i64());
        assert!(factors["recent_authentication"]["expires_at_ms"].is_i64());
    }
    factors.clone()
}

pub fn keys(value: &Value, expected: &[&str]) {
    let object = value.as_object().expect("固定对象投影");
    assert_eq!(object.len(), expected.len());
    for name in expected {
        assert!(object.contains_key(*name), "缺字段 {name}");
    }
}

pub fn no_secrets(reply: &Reply, login: &Login, additional: &[&str]) {
    let body = std::str::from_utf8(&reply.body).unwrap();
    let pair = login.cookie.split_once('=').unwrap().1;
    let token = pair.split_once('~').unwrap().0;
    for secret in [token, login.csrf.as_str(), "fixture-v1", "fixture-v2"] {
        assert!(!body.contains(secret));
    }
    for secret in additional {
        assert!(!body.contains(secret));
    }
}
