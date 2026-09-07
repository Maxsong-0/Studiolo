//! HTTP 专用 Auth 入口的本地夹具；只使用虚构种子和真实临时 SQLite。

use super::*;
use hmac::{Hmac, Mac};
use std::sync::Arc;

pub(super) const START: i64 = 1_000_000;

pub(super) fn fixture(version: u8, idle: u64, absolute: u64) -> Harness {
    let mut h = Harness::new(idle, absolute);
    for next in 3..=version {
        let migrate = if next == 3 {
            SessionManager::migrate_v2
        } else {
            SessionManager::migrate_v3
        };
        h.manager = Arc::new(
            migrate(
                &h.root,
                config(idle, absolute),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap(),
        );
    }
    h
}

pub(super) fn enrollment(h: &Harness) -> TotpEnrollment {
    Totp::attach(h.manager.clone(), seed()).unwrap().1
}

pub(super) fn seed() -> TotpSeed {
    TotpSeed::new("http-contract-only".into(), *b"fictional-test-seed!").unwrap()
}

pub(super) fn code() -> TotpCode {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"fictional-test-seed!").unwrap();
    mac.update(&(START as u64 / 30_000).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 15);
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    TotpCode::parse(format!("{:06}", value % 1_000_000)).unwrap()
}

pub(super) fn enrolled(version: u8, idle: u64, absolute: u64) -> Harness {
    let h = fixture(version, idle, absolute);
    enrollment(&h).enroll(code()).unwrap();
    h
}
