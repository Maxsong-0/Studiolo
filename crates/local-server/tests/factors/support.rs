//! 仅虚构秘密、规范主人配置和独立OTP计算；不复用或重复编译旧测试集合。

use hmac::{Hmac, Mac};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::atomic::{AtomicI64, Ordering},
};
use studiolo_auth::{
    OwnerBinding, SessionClock, SessionConfig, SessionError, SessionPolicy, TotpSeed,
};

pub const ORIGIN: &str = "https://studiolo.example.com";
pub const HOST: &str = "studiolo.example.com";
pub const STEP_UP: &str = "/api/v1/auth/step-up";
pub const CODES: &str = "/api/v1/auth/recovery-codes";
pub const RECOVER: &str = "/api/v1/auth/factor/recover";
pub const SESSIONS: &str = "/api/v1/sessions";
pub const OWN: &str = "/api/v1/session";
pub const REVOKE: &str = "/api/v1/sessions/revoke";
pub const OLD: [u8; 20] = *b"12345678901234567890";
pub const NEW: [u8; 20] = [73; 20];

pub struct Clock(pub AtomicI64);
impl SessionClock for Clock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

pub struct Login {
    pub cookie: String,
    pub csrf: String,
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

pub fn seed(replacement: bool) -> TotpSeed {
    TotpSeed::new(
        if replacement {
            "fixture-v2"
        } else {
            "fixture-v1"
        }
        .into(),
        if replacement { NEW } else { OLD },
    )
    .unwrap()
}

pub fn otp(secret: &[u8; 20], now: i64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(secret).unwrap();
    mac.update(&((now / 30_000) as u64).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 15);
    let value = u32::from_be_bytes(hash[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

pub fn wrong(secret: &[u8; 20], now: i64) -> String {
    let valid = [now - 30_000, now, now + 30_000].map(|time| otp(secret, time));
    (0..10)
        .map(|value| format!("{value:06}"))
        .find(|value| !valid.contains(value))
        .unwrap()
}

pub fn mode(path: &Path, permissions: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(permissions)).unwrap();
}
