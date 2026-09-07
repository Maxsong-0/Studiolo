//! 独立计算虚构 OTP；实际格式与匹配算法的 RFC 向量由算法契约负责。

use super::*;
use hmac::{Hmac, Mac};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

pub(super) const START: i64 = 1_000_000;
pub(super) const NEXT: i64 = 1_060_000;
pub(super) const SEED: [u8; 20] = *b"fictional-test-seed!";
pub(super) const SEED_REF: &str = "test-only-owner-totp-1";

pub(super) fn seed() -> TotpSeed {
    TotpSeed::new(SEED_REF.to_owned(), SEED).unwrap()
}

pub(super) fn code_text(now: i64) -> String {
    let step = u64::try_from(now / 30_000).unwrap();
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&SEED).unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 15);
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

pub(super) fn code(now: i64) -> TotpCode {
    TotpCode::parse(code_text(now)).unwrap()
}

pub(super) fn wrong_code(now: i64) -> TotpCode {
    let valid = [
        code_text(now - 30_000),
        code_text(now),
        code_text(now + 30_000),
    ];
    let wrong = (0..10)
        .map(|value| format!("{value:06}"))
        .find(|candidate| !valid.contains(candidate))
        .unwrap();
    TotpCode::parse(wrong).unwrap()
}

pub(super) fn request<'a>(
    identity: &'a crate::AccessIdentity,
    issued: &'a IssuedSession,
    source: u8,
) -> TotpRequest<'a> {
    TotpRequest {
        identity,
        token: &issued.token,
        csrf: &issued.csrf,
        source: IpAddr::V4(Ipv4Addr::new(192, 0, 2, source)),
    }
}

pub(super) struct TotpHarness {
    pub h: Harness,
    pub totp: Totp,
    pub enrollment: TotpEnrollment,
    idle: u64,
    absolute: u64,
}

impl TotpHarness {
    pub fn new(idle: u64, absolute: u64) -> Self {
        let mut h = Harness::new(idle, absolute);
        h.manager = Arc::new(
            SessionManager::migrate_v2(
                &h.root,
                config(idle, absolute),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap(),
        );
        let (totp, enrollment) = Totp::attach(h.manager.clone(), seed()).unwrap();
        Self {
            h,
            totp,
            enrollment,
            idle,
            absolute,
        }
    }

    pub fn enrolled(idle: u64, absolute: u64) -> Self {
        let value = Self::new(idle, absolute);
        value.enrollment.enroll(code(START)).unwrap();
        value.h.clock.set(NEXT);
        value
    }

    pub fn reopen(&self) -> (Arc<SessionManager>, Totp, TotpEnrollment) {
        let manager = Arc::new(self.h.reopen(self.idle, self.absolute));
        let (totp, enrollment) = Totp::attach(manager.clone(), seed()).unwrap();
        (manager, totp, enrollment)
    }
}

pub(super) fn text_rows(manager: &SessionManager, sql: &str) -> Vec<String> {
    let connection = manager.database.connection.lock().unwrap();
    let mut statement = connection.prepare(sql).unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
