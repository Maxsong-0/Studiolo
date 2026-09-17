use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_vault_broker::FsVaultBroker;
#[path = "process_support.rs"]
mod process_support;
#[path = "unsafe_target_process.rs"]
mod unsafe_target;

fn binary() -> &'static Path {
    // 原生发布包使用校验过的生产程序；覆盖仅由测试读取，不进入产品配置。
    static BINARY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BINARY
        .get_or_init(|| {
            let path = std::env::var_os("STUDIOLO_TEST_MAINTAIN_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_studiolo-maintain")));
            assert!(
                path.is_absolute() && path.is_file(),
                "测试产品二进制必须为已存在的绝对文件路径"
            );
            path
        })
        .as_path()
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new(audit: bool) -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let root = temp.path().to_path_buf();
        for name in ["config", "vault", "assets", "writer", "audit", "auth"] {
            let path = root.join(name);
            fs::create_dir(&path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = root.join("config/owner.json");
        fs::write(&config, serde_json::to_vec(&serde_json::json!({
            "version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture","subject":"CONFIG_IS_NOT_OPERATOR","email":"fixture@example.com"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"FIXTURE_UNUSED_SEED_REFERENCE"
        })).unwrap()).unwrap();
        fs::set_permissions(config, fs::Permissions::from_mode(0o600)).unwrap();
        drop(FsVaultBroker::open(&root.join("vault"), &root.join("writer")).unwrap());
        if audit {
            drop(SqliteAuditLedger::open(&root.join("audit")).unwrap());
        }
        Self { _temp: temp, root }
    }
    fn args(&self, action: &str) -> Vec<OsString> {
        vec![
            action.into(),
            "--config".into(),
            self.root.join("config/owner.json").into_os_string(),
            "--vault".into(),
            self.root.join("vault").into_os_string(),
            "--assets".into(),
            self.root.join("assets").into_os_string(),
            "--writer-data".into(),
            self.root.join("writer").into_os_string(),
        ]
    }
}

#[test]
fn real_cli_help_is_safe_but_pipe_and_unknown_flags_cannot_open_stores() {
    let mut help = Command::new(binary());
    help.arg("--help");
    let (status, output) = process_support::output(help, None);
    assert!(status.success(), "{output}");
    assert!(output.contains("cancel-maintenance"));
    let fixture = Fixture::new(false);
    let mut direct = Command::new(binary());
    direct.args(fixture.args("inspect"));
    let (status, output) = process_support::output(direct, None);
    assert!(!status.success());
    assert!(output.contains("前台控制终端"), "{output}");
    assert!(!output.contains("CONFIG_IS_NOT_OPERATOR"));
    let mut unknown = Command::new(binary());
    unknown.args(fixture.args("audit-flush")).arg("--yes");
    let (status, output) = process_support::output(unknown, None);
    assert!(!status.success());
    assert!(output.contains("维护参数无效"), "{output}");
    for name in ["auth", "audit"] {
        assert_eq!(fs::read_dir(fixture.root.join(name)).unwrap().count(), 0);
    }
}

#[test]
fn real_pty_inspect_reports_verified_state_without_creating_audit() {
    let fixture = Fixture::new(false);
    let (status, output) = process_support::output(
        process_support::script(binary(), &fixture.args("inspect")),
        None,
    );
    assert!(status.success(), "{output}");
    assert!(output.contains("\"audit_available\":false"), "{output}");
    assert!(output.contains("\"command\":\"inspect\""));
    assert!(!output.contains("CONFIG_IS_NOT_OPERATOR"));
    assert_eq!(fs::read_dir(fixture.root.join("audit")).unwrap().count(), 0);
}

#[test]
fn real_pty_wrong_confirmation_refuses_and_a_live_owner_is_not_stopped() {
    let fixture = Fixture::new(true);
    let (_, output) = process_support::output(
        process_support::script(binary(), &fixture.args("audit-flush")),
        Some(b"incorrect\n"),
    );
    assert!(output.contains("维护确认已取消、超时或不匹配"), "{output}");
    let ledger = SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap();
    assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    let owner =
        FsVaultBroker::open_existing(&fixture.root.join("vault"), &fixture.root.join("writer"))
            .unwrap();
    let (_, output) = process_support::output(
        process_support::script(binary(), &fixture.args("inspect")),
        None,
    );
    assert!(output.contains("已有 owner"), "{output}");
    assert!(!owner.write_status().unwrap().unresolved);
}
