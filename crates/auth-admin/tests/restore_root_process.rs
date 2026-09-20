//! 仅显式 --ignored 在 root 独立网络命名空间运行；固定 RFC 之外的虚构种子。
//! 不加载任何真实配置。目标机可通过 TEST_BINARY 指定主线复制的产品路径。

use hmac::{Hmac, Mac};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::Arc,
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{
    OsSessionRandom, SessionClock, SessionError, SessionManager, SystemSessionClock, Totp,
    TotpCode, TotpSeed,
};
use studiolo_host_config::Config;

const BASE: &str = "/var/lib/studiolo-dr-tests";
const SEED: [u8; 20] = [42; 20];

// 仅虚构夹具：持久时间地板领先真实时钟一小时，离线撤权不得观察运行时钟。
struct FutureClock;

impl SessionClock for FutureClock {
    fn now_unix_ms(&self) -> Result<i64, SessionError> {
        SystemSessionClock
            .now_unix_ms()?
            .checked_add(3_600_000)
            .ok_or(SessionError::ClockRollback)
    }
}

fn mode(path: &Path, value: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(value)).unwrap();
}

fn run(binary: &Path, root: &Path, input: &[u8], id: &str) -> Output {
    let mut child = Command::new(binary)
        .args(["restore-invalidate", "--config"])
        .arg(root.join("owner.json"))
        .arg("--restore-root")
        .arg(root)
        .args(["--restore-id", id])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env(
            "STUDIOLO_TOTP_SEED_HEX",
            "fictional-environment-must-be-ignored",
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(input);
    let result = child.wait_with_output().unwrap();
    for raw in [&result.stdout, &result.stderr] {
        let text = String::from_utf8_lossy(raw);
        assert!(!text.contains(&"2a".repeat(20)));
        assert!(!text.contains("fictional-environment-must-be-ignored"));
        assert!(!text.contains("fictional/private-marker"));
    }
    result
}

fn initialize(root: &Path) {
    for path in [root.to_path_buf(), root.join("auth"), root.join("audit")] {
        fs::DirBuilder::new().mode(0o700).create(path).unwrap();
    }
    let document = serde_json::json!({"version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
        "owner":{"issuer":"https://fictional.cloudflareaccess.com","audience":"fictional-cli",
            "subject":"fictional-owner","email":"owner@example.invalid"},
        "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"studiolo-human-totp-v2"});
    fs::write(
        root.join("owner.json"),
        serde_json::to_vec(&document).unwrap(),
    )
    .unwrap();
    mode(&root.join("owner.json"), 0o600);
    let config = Config::load(&root.join("owner.json")).unwrap();
    let audit = Arc::new(SqliteAuditLedger::open(config.audit_directory()).unwrap());
    let sessions = Arc::new(
        SessionManager::migrate_v3(
            config.auth_directory(),
            config.sessions().unwrap(),
            audit,
            Arc::new(FutureClock),
            Arc::new(OsSessionRandom),
        )
        .unwrap(),
    );
    let (_, enrollment) = Totp::attach(
        sessions,
        TotpSeed::new("studiolo-human-totp-v2".into(), SEED).unwrap(),
    )
    .unwrap();
    let step = u64::try_from(FutureClock.now_unix_ms().unwrap() / 30_000).unwrap();
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&SEED).unwrap();
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[19] & 15);
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    enrollment
        .enroll(TotpCode::parse(format!("{:06}", value % 1_000_000)).unwrap())
        .unwrap();
}

#[test]
#[ignore = "仅主线在获准目标的root独立网络命名空间执行纯虚构数据"]
fn isolated_root_cli_requires_copy_layout_and_invalidates_once() {
    assert_eq!(rustix::process::getuid().as_raw(), 0);
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    assert_ne!(
        fs::metadata("/proc/self/ns/net").unwrap().ino(),
        fs::metadata("/proc/1/ns/net").unwrap().ino()
    );
    let binary = std::env::var_os("STUDIOLO_AUTH_ADMIN_TEST_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_studiolo-auth-admin")));
    assert!(binary.is_absolute() && binary.is_file());
    match fs::DirBuilder::new().mode(0o700).create(BASE) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => panic!("不能建立虚构测试根: {error}"),
    }
    let metadata = fs::symlink_metadata(BASE).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    assert_eq!((metadata.uid(), metadata.mode() & 0o7777), (0, 0o700));
    let run_dir = tempfile::Builder::new()
        .prefix("fictional-t094-")
        .tempdir_in(BASE)
        .unwrap();
    mode(run_dir.path(), 0o700);
    let root = run_dir.path().join("human-work");
    initialize(&root);
    let seed = "2a".repeat(20);
    let id = "fictional-restore-1";
    let database = root.join("auth/auth.sqlite3");
    let before = fs::read(&database).unwrap();
    for input in [
        vec![b'a'; 39],
        vec![b'A'; 40],
        [seed.as_bytes(), b"\n"].concat(),
    ] {
        assert!(!run(&binary, &root, &input, id).status.success());
        assert_eq!(fs::read(&database).unwrap(), before);
    }
    assert!(
        !run(&binary, &root, seed.as_bytes(), "fictional/private-marker")
            .status
            .success()
    );
    let config = root.join("owner.json");
    mode(&config, 0o644);
    assert!(!run(&binary, &root, seed.as_bytes(), id).status.success());
    mode(&config, 0o600);
    let hardlink = run_dir.path().join("fixture-hardlink");
    fs::hard_link(&database, &hardlink).unwrap();
    assert!(!run(&binary, &root, seed.as_bytes(), id).status.success());
    fs::remove_file(hardlink).unwrap();
    let original = run_dir.path().join("original-owner.json");
    fs::rename(&config, &original).unwrap();
    std::os::unix::fs::symlink(&original, &config).unwrap();
    assert!(!run(&binary, &root, seed.as_bytes(), id).status.success());
    fs::remove_file(&config).unwrap();
    fs::rename(original, &config).unwrap();
    mode(&root, 0o755);
    assert!(!run(&binary, &root, seed.as_bytes(), id).status.success());
    mode(&root, 0o700);
    assert_eq!(fs::read(&database).unwrap(), before);
    let success = run(&binary, &root, seed.as_bytes(), id);
    assert!(
        success.status.success(),
        "纯虚构离线CLI失败: {}",
        String::from_utf8_lossy(&success.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(result.as_object().unwrap().len(), 5);
    assert_eq!(result["audit_confirmed"], true);
    assert_eq!(result["revoked_sessions"], 0);
    assert_eq!(result["cleared_l3"], 0);
    assert_eq!(result["invalidated_recovery_codes"], 0);
    assert!(result["epoch"].as_i64().unwrap() > 0);
    let repeated = run(&binary, &root, seed.as_bytes(), id);
    assert!(repeated.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&repeated.stdout).unwrap(),
        result
    );
}

#[test]
fn ordinary_process_rejects_restore_without_echoing_input_or_arguments() {
    if rustix::process::getuid().as_raw() == 0 {
        return;
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_studiolo-auth-admin"));
    let root = Path::new("/var/lib/studiolo-dr-tests/fictional-no-access/human-work");
    let result = run(binary, root, b"fictional-private-stdin", "fixture");
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains("fictional-private-stdin"));
}
