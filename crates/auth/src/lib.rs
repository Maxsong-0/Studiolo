//! 真实 Access、持久会话与因子/恢复码基础，不开放 HTTP、真实登记或 Vault 写入。

#[cfg(all(feature = "production", feature = "test-support"))]
compile_error!("STUDIOLO_AUTH_TEST_FIXTURE_FORBIDDEN_IN_PRODUCTION");

mod access;
mod session;

pub use access::{AccessConfig, AccessError, AccessIdentity, AccessVerifier};
pub use session::{
    CsrfToken, FactorRuntime, FactorState, IssuedSession, OsSessionRandom, OwnerBinding,
    PreparedTotpFactor, RecentAuthentication, Recovery, RecoveryCode, RecoveryCodes,
    RetentionReport, RetentionWork, Revocation, SensitiveAction, SessionClock, SessionConfig,
    SessionError, SessionId, SessionInfo, SessionManager, SessionPolicy, SessionRandom,
    SessionState, SessionToken, SystemSessionClock, Totp, TotpCode, TotpEnrollment,
    TotpEnrollmentState, TotpRequest, TotpSeed,
};

#[cfg(feature = "test-support")]
pub use access::fixture_verifier;
