//! 只经公开 Auth 与 Audit Interface 生成启动夹具；种子完全虚构。

use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{
    OsSessionRandom, OwnerBinding, SessionConfig, SessionError, SessionManager, SessionPolicy,
    SystemSessionClock, Totp, TotpCode, TotpEnrollmentState, TotpSeed,
};

pub const ORIGIN: &str = "https://startup.example.invalid";
pub const ISSUER: &str = "https://studiolo-startup-fixture.cloudflareaccess.com";
pub const AUDIENCE: &str = "fixture-private-audience";
pub const SUBJECT: &str = "fixture-private-subject";
pub const EMAIL: &str = "owner-private-canary@example.invalid";
pub const REFERENCE: &str = "fixture-private-seed-reference";
const SEED: [u8; 20] = *b"12345678901234567890";

pub struct Fixture {
    _temporary: tempfile::TempDir,
    pub root: PathBuf,
    pub auth: PathBuf,
    pub audit: PathBuf,
    pub config: PathBuf,
    pub vault: PathBuf,
    pub web: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        mode(&root, 0o700);
        let fixture = Self {
            auth: root.join("auth"),
            audit: root.join("audit"),
            config: root.join("private/owner.json"),
            vault: root.join("vault"),
            web: root.join("web"),
            root,
            _temporary: temporary,
        };
        for path in [
            &fixture.auth,
            &fixture.audit,
            &fixture.vault,
            &fixture.web,
            fixture.config.parent().unwrap(),
        ] {
            fs::create_dir(path).unwrap();
            mode(path, 0o700);
        }
        fs::create_dir(fixture.web.join("assets")).unwrap();
        fs::write(
            fixture.vault.join("fixture.md"),
            "# 虚构启动夹具\n只读正文\n",
        )
        .unwrap();
        fs::write(
            fixture.web.join("index.html"),
            "<!doctype html><title>启动夹具</title>",
        )
        .unwrap();
        fs::write(fixture.web.join("assets/app.js"), "const fixture = true;").unwrap();
        fixture.write_config(&fixture.value());
        fixture
    }

    pub fn value(&self) -> Value {
        json!({
            "version": 1, "auth_directory": self.auth, "audit_directory": self.audit,
            "owner": {"issuer": ISSUER, "audience": AUDIENCE, "subject": SUBJECT, "email": EMAIL},
            "idle_seconds": 600, "absolute_seconds": 1800, "seed_ref": REFERENCE
        })
    }

    pub fn write_config(&self, value: &Value) {
        fs::write(&self.config, serde_json::to_vec(value).unwrap()).unwrap();
        mode(&self.config, 0o600);
    }

    pub fn legacy_args(&self) -> Vec<String> {
        vec![
            "--vault".into(),
            self.vault.to_str().unwrap().into(),
            "--assets".into(),
            self.web.to_str().unwrap().into(),
            "--label".into(),
            "虚构启动夹具".into(),
        ]
    }

    pub fn args(&self) -> Vec<String> {
        let mut args = self.legacy_args();
        args.extend([
            "--session-config".into(),
            self.config.to_str().unwrap().into(),
            "--public-origin".into(),
            ORIGIN.into(),
        ]);
        args
    }

    pub fn reject(&self, args: Vec<String>) {
        super::process::reject(
            &self.root,
            args,
            &[
                ISSUER,
                AUDIENCE,
                SUBJECT,
                EMAIL,
                REFERENCE,
                "another-private-subject",
                "private-unknown-canary",
                "invalid-environment-secret-canary",
                self.root.to_str().unwrap(),
            ],
        );
    }

    pub fn initialize(&self, version: u8, enrolled: bool, pending: bool) {
        let ledger = Arc::new(SqliteAuditLedger::open(&self.audit).unwrap());
        let config = session_config();
        let manager = if version == 2 {
            SessionManager::open(
                &self.auth,
                config,
                ledger,
                Arc::new(SystemSessionClock),
                Arc::new(OsSessionRandom),
            )
        } else {
            SessionManager::migrate_v3(
                &self.auth,
                config,
                ledger,
                Arc::new(SystemSessionClock),
                Arc::new(OsSessionRandom),
            )
        }
        .unwrap();
        if enrolled || pending {
            let (_, enrollment) = Totp::attach(
                Arc::new(manager),
                TotpSeed::new(REFERENCE.into(), SEED).unwrap(),
            )
            .unwrap();
            if pending {
                mode(&self.audit.join("audit.sqlite3"), 0o400);
            }
            let result = enrollment.enroll(current_code());
            if pending {
                mode(&self.audit.join("audit.sqlite3"), 0o600);
                assert_eq!(result.unwrap_err(), SessionError::AuditUnavailable);
            } else {
                result.unwrap();
            }
        }
    }

    pub fn enrollment_state(&self) -> Result<TotpEnrollmentState, SessionError> {
        let manager = SessionManager::open_existing(
            &self.auth,
            session_config(),
            Arc::new(SqliteAuditLedger::open_existing(&self.audit).unwrap()),
            Arc::new(SystemSessionClock),
            Arc::new(OsSessionRandom),
        )?;
        let (_, enrollment) = Totp::attach(
            Arc::new(manager),
            TotpSeed::new(REFERENCE.into(), SEED).unwrap(),
        )?;
        enrollment.state()
    }

    pub fn empty_stores(&self) {
        assert_eq!(fs::read_dir(&self.auth).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&self.audit).unwrap().count(), 0);
    }
}

pub fn mode(path: &Path, permissions: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(permissions)).unwrap();
}

fn session_config() -> SessionConfig {
    SessionConfig::new(
        OwnerBinding::new(ISSUER.into(), AUDIENCE.into(), SUBJECT.into(), EMAIL.into()).unwrap(),
        SessionPolicy::new(600, 1800).unwrap(),
    )
}

fn current_code() -> TotpCode {
    let step = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&SEED).unwrap();
    mac.update(&step.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 15);
    let value = u32::from_be_bytes(hash[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    TotpCode::parse(format!("{:06}", value % 1_000_000)).unwrap()
}
