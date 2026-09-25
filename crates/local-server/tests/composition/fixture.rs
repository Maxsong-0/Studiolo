//! 内部组合测试仅替换取钥Adapter，其余使用真实配置文件、双库和路由。

#[path = "../loopback_port/mod.rs"]
#[allow(dead_code)]
mod loopback_port;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{
    OsSessionRandom, SessionManager, SystemSessionClock, Totp, TotpCode, TotpSeed,
};
use studiolo_host_config::Config;

pub(super) struct Fixture {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub config: PathBuf,
    pub port: u16,
    // 端口占位与夹具同寿（T-171）：产品在进程内绑定同一端口，期间别的进程拿不走它。
    _reservation: loopback_port::Reservation,
}

impl Fixture {
    pub fn new(enrolled: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        Self::from_directory(enrolled, temp)
    }

    pub fn in_tmp(enrolled: bool) -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        Self::from_directory(enrolled, temp)
    }

    fn from_directory(enrolled: bool, temp: tempfile::TempDir) -> Self {
        let root = temp.path().canonicalize().unwrap();
        for path in [
            &root,
            &root.join("config"),
            &root.join("auth"),
            &root.join("audit"),
            &root.join("vault"),
            &root.join("web"),
        ] {
            if !path.exists() {
                fs::create_dir(path).unwrap();
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(root.join("vault/note.md"), "# synthetic note\n").unwrap();
        fs::write(
            root.join("web/index.html"),
            "<!doctype html><title>fixture</title>",
        )
        .unwrap();
        let config = root.join("config/owner.json");
        let value = serde_json::json!({"version":1,"auth_directory":root.join("auth"),
            "audit_directory":root.join("audit"),"owner":{"issuer":"https://studiolo.cloudflareaccess.com",
            "audience":"example-audience","subject":"fictional-person","email":"maxsong@example.com"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"synthetic-seed"});
        fs::write(&config, serde_json::to_vec(&value).unwrap()).unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
        let settings = Config::load(&config).unwrap();
        let ledger = Arc::new(SqliteAuditLedger::open(settings.audit_directory()).unwrap());
        let manager = Arc::new(
            SessionManager::migrate_v3(
                settings.auth_directory(),
                settings.sessions().unwrap(),
                ledger,
                Arc::new(SystemSessionClock),
                Arc::new(OsSessionRandom),
            )
            .unwrap(),
        );
        if enrolled {
            let (_, enrollment) = Totp::attach(
                manager,
                TotpSeed::new("synthetic-seed".into(), *b"12345678901234567890").unwrap(),
            )
            .unwrap();
            enrollment.enroll(code()).unwrap();
        }
        let reservation = loopback_port::Reservation::new();
        Self {
            _temp: temp,
            root,
            config,
            port: reservation.address.port(),
            _reservation: reservation,
        }
    }

    pub fn options(&self) -> crate::options::Options {
        crate::options::parse(
            [
                "--vault".into(),
                self.root.join("vault").to_str().unwrap().into(),
                "--assets".into(),
                self.root.join("web").to_str().unwrap().into(),
                "--port".into(),
                self.port.to_string(),
                "--session-config".into(),
                self.config.to_str().unwrap().into(),
                "--public-origin".into(),
                "https://studiolo.example.com".into(),
            ]
            .into_iter(),
        )
        .unwrap()
        .unwrap()
    }

    pub fn unbound(&self) {
        let socket =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, self.port)).unwrap();
        drop(socket);
    }
}

fn code() -> TotpCode {
    use hmac::{Hmac, Mac};
    let step = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(b"12345678901234567890").unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 15) as usize;
    let number = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    TotpCode::parse(format!("{:06}", number % 1_000_000)).unwrap()
}
