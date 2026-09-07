//! 只经公开HTTP协议构造虚构登录与因子动作；真实来源来自Unix服务自身。

use super::{
    client::{Client, Reply},
    fixture::Fixture,
    support::*,
    test_support::{claims, signed},
};
use axum::http::Request;

impl Fixture {
    pub fn request(
        &self,
        method: &str,
        path: &str,
        login: Option<&Login>,
        body: &str,
    ) -> Request<Vec<u8>> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", HOST)
            .header("origin", ORIGIN)
            .header("cf-access-jwt-assertion", signed(&claims()))
            .header("cf-connecting-ip", "192.0.2.1")
            .header("x-studiolo-request", "1")
            .header("sec-fetch-site", "same-origin")
            .header("sec-fetch-mode", "cors")
            .header("sec-fetch-dest", "empty")
            .header("content-type", "application/json");
        if let Some(login) = login {
            request = request
                .header("cookie", &login.cookie)
                .header("x-csrf-token", &login.csrf);
        }
        request.body(body.as_bytes().to_vec()).unwrap()
    }

    pub async fn send(&self, request: Request<Vec<u8>>, status: u16) -> Reply {
        let reply = Client::connect(&self.socket).await.send(request).await;
        assert_eq!(reply.status, status);
        reply.secure();
        reply
    }

    pub async fn issue(&self) -> Login {
        let response = self
            .send(self.request("POST", SESSIONS, None, "{}"), 201)
            .await;
        let raw = response.headers["set-cookie"].to_str().unwrap();
        for attribute in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/"] {
            assert!(raw.contains(attribute));
        }
        let cookie = raw.split(';').next().unwrap().to_owned();
        let body = response.json();
        let csrf = body["csrf_token"].as_str().unwrap().to_owned();
        assert!(cookie.ends_with(&format!("~{csrf}")));
        Login { cookie, csrf }
    }

    pub async fn elevate(&self, login: &Login, replacement: bool) {
        let code = otp(if replacement { &NEW } else { &OLD }, self.now());
        let response = self
            .send(
                self.request(
                    "POST",
                    STEP_UP,
                    Some(login),
                    &format!("{{\"code\":\"{code}\"}}"),
                ),
                200,
            )
            .await;
        response.kept_cookie();
        let body = response.json();
        assert_eq!(body.as_object().unwrap().len(), 2);
        assert_eq!(body["verified_at_ms"], self.now());
        assert!(body["expires_at_ms"].as_i64().unwrap() <= self.now() + 300_000);
    }
}
