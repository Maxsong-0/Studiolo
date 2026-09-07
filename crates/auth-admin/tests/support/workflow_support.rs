//! 工作流使用真实临时双库、完整配置加载和独立计算的虚构验证码。

use super::*;
use hmac::{Hmac, Mac};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditRecord, SqliteAuditLedger};
use studiolo_auth::{OsSessionRandom, SessionManager, SystemSessionClock, Totp, TotpSeed};

pub(super) const SEED: [u8; 20] = *b"fictional-test-seed!";
const REFERENCE: &str = "test-only-admin-totp";

pub(super) struct Fixture {
    _directory: tempfile::TempDir,
    pub config: PathBuf,
    pub auth: PathBuf,
    pub audit: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        mode(&root, 0o700);
        let auth = root.join("auth");
        let audit = root.join("audit");
        for path in [&auth, &audit] {
            fs::create_dir(path).unwrap();
            mode(path, 0o700);
        }
        let config = root.join("owner.json");
        let value = serde_json::json!({
            "version":1,"auth_directory":auth,"audit_directory":audit,
            "owner":{"issuer":"https://fixture.cloudflareaccess.com",
                "audience":"fictional-admin-audience","subject":"fictional-owner",
                "email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":REFERENCE
        });
        fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
        mode(&config, 0o600);
        Self {
            _directory: directory,
            config,
            auth,
            audit,
        }
    }

    pub fn load(&self) -> Config {
        Config::load(&self.config).unwrap()
    }

    pub fn run(
        &self,
        command: Command,
        read_code: impl FnOnce() -> Result<TotpCode, TerminalError>,
    ) -> Result<&'static str, AdminError> {
        let seed = (!matches!(command, Command::Migrate(_))).then(seed);
        execute(command, self.load(), seed, read_code)
    }

    pub fn initialize(&self) {
        self.run(Command::Migrate(3), never_prompt).unwrap();
    }

    pub fn state(&self) -> Result<TotpEnrollmentState, AdminError> {
        let config = self.load();
        let audit = SqliteAuditLedger::open_existing(config.audit_directory())
            .map_err(|_| AdminError::Audit)?;
        let manager = SessionManager::open_existing(
            config.auth_directory(),
            config.sessions()?,
            Arc::new(audit),
            Arc::new(SystemSessionClock),
            Arc::new(OsSessionRandom),
        )?;
        let (_, enrollment) = Totp::attach(Arc::new(manager), seed())?;
        enrollment.state().map_err(AdminError::from)
    }

    pub fn events(&self, action: &str) -> Vec<AuditRecord> {
        SqliteAuditLedger::open_existing(&self.audit)
            .unwrap()
            .query(&AuditFilter::default().with_action(action))
            .unwrap()
    }

    pub fn empty(&self) {
        assert_eq!(fs::read_dir(&self.auth).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&self.audit).unwrap().count(), 0);
    }
}

pub(super) fn seed() -> TotpSeed {
    TotpSeed::new(REFERENCE.into(), SEED).unwrap()
}

pub(super) fn other_seed() -> TotpSeed {
    TotpSeed::new(REFERENCE.into(), [91; 20]).unwrap()
}

pub(super) fn mode(path: &Path, permissions: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(permissions)).unwrap();
}

pub(super) fn never_prompt() -> Result<TotpCode, TerminalError> {
    panic!("此路径不得索取验证码")
}

fn step() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30
}

fn digits(step: u64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&SEED).unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 15);
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}

pub(super) fn current_code() -> Result<TotpCode, TerminalError> {
    Ok(TotpCode::parse(digits(step())).unwrap())
}

pub(super) fn wrong_code() -> Result<TotpCode, TerminalError> {
    let step = step();
    // 多排除相邻两步，避免测试恰遇正常时间步切换时把拒绝夹具误作正确码。
    let valid: Vec<_> = (step.saturating_sub(2)..=step + 2).map(digits).collect();
    let wrong = (0..10)
        .map(|n| format!("{n:06}"))
        .find(|candidate| !valid.contains(candidate))
        .unwrap();
    Ok(TotpCode::parse(wrong).unwrap())
}
