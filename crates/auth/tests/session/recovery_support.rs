//! 只新增局部夹具；独立算虚构 OTP，不重复算法 RFC 向量或迁移夹具。

use super::*;
use hmac::{Hmac, Mac};
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

pub(super) const START: i64 = 1_000_000;
pub(super) const NEXT: i64 = 1_060_000;
pub(super) const CHANGE: i64 = NEXT + 60_000;
pub(super) const OLD: [u8; 20] = *b"fictional-test-seed!";
pub(super) const NEW: [u8; 20] = [73; 20];

pub(super) fn seed(bytes: [u8; 20]) -> TotpSeed {
    let reference = if bytes == OLD {
        "test-only-old"
    } else {
        "test-only-new"
    };
    TotpSeed::new(reference.into(), bytes).unwrap()
}

pub(super) fn otp_text(bytes: &[u8; 20], now: i64) -> String {
    let step = u64::try_from(now / 30_000).unwrap();
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(bytes).unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 15);
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

pub(super) fn otp(bytes: &[u8; 20], now: i64) -> TotpCode {
    TotpCode::parse(otp_text(bytes, now)).unwrap()
}

pub(super) fn wrong_otp(now: i64) -> TotpCode {
    let valid = [
        otp_text(&NEW, now - 30_000),
        otp_text(&NEW, now),
        otp_text(&NEW, now + 30_000),
    ];
    let wrong = (0..10)
        .map(|n| format!("{n:06}"))
        .find(|s| !valid.contains(s))
        .unwrap();
    TotpCode::parse(wrong).unwrap()
}

pub(super) fn request<'a>(
    owner: &'a crate::AccessIdentity,
    issued: &'a IssuedSession,
    source: u8,
) -> TotpRequest<'a> {
    TotpRequest {
        identity: owner,
        token: &issued.token,
        csrf: &issued.csrf,
        source: IpAddr::V4(Ipv4Addr::new(192, 0, 2, source)),
    }
}

pub(super) fn copy_code(codes: &RecoveryCodes, slot: usize) -> RecoveryCode {
    RecoveryCode::parse(codes.codes()[slot].expose_secret().to_owned()).unwrap()
}

pub(super) fn invalid_code(codes: &RecoveryCodes) -> RecoveryCode {
    let text = codes.codes()[0].expose_secret();
    let replacement = if text.ends_with('0') { '1' } else { '0' };
    RecoveryCode::parse(format!("{}{replacement}", &text[..54])).unwrap()
}

pub(super) struct RecoveryHarness {
    pub h: Harness,
    pub totp: Totp,
    pub recovery: Recovery,
    idle: u64,
    absolute: u64,
}

impl RecoveryHarness {
    pub fn new(idle: u64, absolute: u64) -> Self {
        let mut h = Harness::new(idle, absolute);
        SessionManager::migrate_v2(
            &h.root,
            config(idle, absolute),
            h.audit.clone(),
            h.clock.clone(),
            h.random.clone(),
        )
        .unwrap();
        h.manager = Arc::new(
            SessionManager::migrate_v3(
                &h.root,
                config(idle, absolute),
                h.audit.clone(),
                h.clock.clone(),
                h.random.clone(),
            )
            .unwrap(),
        );
        let (totp, enrollment) = Totp::attach(h.manager.clone(), seed(OLD)).unwrap();
        enrollment.enroll(otp(&OLD, START)).unwrap();
        h.clock.set(NEXT);
        let recovery = Recovery::attach(h.manager.clone()).unwrap();
        Self {
            h,
            totp,
            recovery,
            idle,
            absolute,
        }
    }

    pub fn ready(&self) -> (IssuedSession, RecoveryCodes) {
        let owner = identity();
        let issued = self.h.manager.issue(&owner).unwrap();
        let req = request(&owner, &issued, 1);
        self.totp.step_up(&req, otp(&OLD, NEXT)).unwrap();
        let codes = self.recovery.rotate_codes(&req).unwrap();
        (issued, codes)
    }

    pub fn prepared(&self) -> PreparedTotpFactor {
        PreparedTotpFactor::prepare(self.h.manager.clone(), seed(NEW)).unwrap()
    }

    pub fn reopen(&self) -> (Arc<SessionManager>, Recovery) {
        let manager = Arc::new(self.h.reopen(self.idle, self.absolute));
        let recovery = Recovery::attach(manager.clone()).unwrap();
        (manager, recovery)
    }
}

pub(super) fn execute(manager: &SessionManager, sql: &str) {
    manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}
