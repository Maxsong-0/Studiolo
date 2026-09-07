//! 通过真实公开 Auth Interface 建库、登记、重开；不直接操作业务 SQL。

#[path = "fixture.rs"]
mod fixture;
pub use fixture::Fixture;

use axum::{body::to_bytes, response::Response};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha1::Sha1;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::atomic::{AtomicI64, Ordering},
};
use studiolo_auth::{
    OwnerBinding, SessionClock, SessionConfig, SessionError, SessionPolicy, TotpCode, TotpSeed,
};

pub const ORIGIN: &str = "https://studiolo.example.com";
pub const OWN: &str = "/api/v1/session";
pub const SESSIONS: &str = "/api/v1/sessions";
pub const REVOKE: &str = "/api/v1/sessions/revoke";
pub const COOKIE: &str = "__Host-studiolo-session";
const SEED: &[u8; 20] = b"12345678901234567890";

pub struct Clock(pub AtomicI64);
impl SessionClock for Clock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

pub struct Issued {
    pub cookie: String,
    pub csrf: String,
    pub token: String,
    pub id: String,
}

pub fn config() -> SessionConfig {
    SessionConfig::new(
        OwnerBinding::new(
            "https://studiolo.cloudflareaccess.com".into(),
            "example-audience".into(),
            "fictional-person".into(),
            "maxsong@example.com".into(),
        )
        .unwrap(),
        SessionPolicy::new(600, 1800).unwrap(),
    )
}

pub fn seed() -> TotpSeed {
    TotpSeed::new("fixture-v1".into(), *SEED).unwrap()
}

pub fn code(now: i64) -> TotpCode {
    let mut mac = Hmac::<Sha1>::new_from_slice(SEED).unwrap();
    mac.update(&((now / 30_000) as u64).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 15);
    let value = u32::from_be_bytes(hash[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    TotpCode::parse(format!("{:06}", value % 1_000_000)).unwrap()
}

pub async fn json(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}

pub fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

pub fn cookie_attributes(value: &str) {
    let lower = value.to_ascii_lowercase();
    for attribute in ["secure", "httponly", "samesite=strict", "path=/"] {
        assert!(
            lower
                .split(';')
                .map(str::trim)
                .any(|part| part == attribute)
        );
    }
    assert!(!lower.contains("domain="));
}

pub fn cleared(response: &Response) {
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    cookie_attributes(cookie);
    assert!(cookie.starts_with(&format!("{COOKIE}=")));
    assert!(cookie.to_ascii_lowercase().contains("max-age=0"));
}

pub fn secure(response: &Response) {
    for (name, expected) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("cross-origin-resource-policy", "same-origin"),
    ] {
        assert_eq!(response.headers()[name], expected);
    }
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("default-src 'none'") && csp.contains("frame-ancestors 'none'"));
    assert!(
        !response
            .headers()
            .keys()
            .any(|key| key.as_str().starts_with("access-control-"))
    );
}
