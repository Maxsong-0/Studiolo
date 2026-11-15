//! 操作屏障覆盖独立连接、嵌套调用与另一进程；不依赖进程内 Mutex。
#![cfg(unix)]
mod support;
use std::{
    io::{BufRead, Read, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};

fn deadline() -> Instant {
    Instant::now() + Duration::from_millis(40)
}

#[test]
fn nested_operations_and_another_instance_hold_the_same_gate() {
    let dir = support::private_dir();
    let first = SqliteAuditLedger::open(dir.path()).unwrap();
    let second = SqliteAuditLedger::open_existing(dir.path()).unwrap();
    let outer = first.enter_operation().unwrap();
    let nested = second.enter_operation().unwrap();
    assert!(first.quiesce_operations(deadline()).is_err());
    drop(outer);
    assert!(first.quiesce_operations(deadline()).is_err());
    drop(nested);
    let frozen = first.quiesce_operations(deadline()).unwrap();
    assert!(second.quiesce_operations(deadline()).is_err());
    drop(frozen);
    let _again = second.enter_operation().unwrap();
}

#[test]
fn process_child() {
    let Some(path) = std::env::var_os("STUDIOLO_OPERATION_TEST_ROOT") else {
        return;
    };
    let ledger = SqliteAuditLedger::open_existing(std::path::Path::new(&path)).unwrap();
    let _operation = ledger.enter_operation().unwrap();
    println!("OPERATION-READY");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
}

#[test]
fn another_process_is_drained_and_crash_releases_the_gate() {
    let dir = support::private_dir();
    let ledger = SqliteAuditLedger::open(dir.path()).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "process_child", "--nocapture"])
        .env("STUDIOLO_OPERATION_TEST_ROOT", dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(output.read_line(&mut line).unwrap() > 0);
        if line.contains("OPERATION-READY") {
            break;
        }
    }
    assert!(ledger.quiesce_operations(deadline()).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    let _frozen = ledger.quiesce_operations(deadline()).unwrap();
}

#[test]
fn changed_gate_identity_or_permissions_are_never_adopted() {
    let dir = support::private_dir();
    let ledger = SqliteAuditLedger::open(dir.path()).unwrap();
    let path = dir.path().join("checkpoint-operations.lock");
    let saved = dir.path().join("saved-lock");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::File::create(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(ledger.enter_operation().is_err());
    assert!(ledger.quiesce_operations(deadline()).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::rename(&saved, &path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ledger.enter_operation().is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let _operation = ledger.enter_operation().unwrap();
}

/// 审计中断（库文件权限被改、目录权限被改、目录被挪走）时，完整操作仍能取得共享许可，
/// 各自走既有的「落待补记、照常读」路径；冻结方必须面对健康的审计库，一律拒绝。
#[test]
fn shared_permits_survive_an_audit_outage_while_freezing_is_refused() {
    use studiolo_audit_ledger::AuditFilter;
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let mode = |path: &std::path::Path, bits| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap();
    };
    let database = root.join("audit.sqlite3");

    mode(&database, 0o400);
    assert!(ledger.query(&AuditFilter::default()).is_err());
    drop(ledger.enter_operation().unwrap());
    assert!(ledger.quiesce_operations(deadline()).is_err());
    mode(&database, 0o600);

    mode(&root, 0o750);
    assert!(ledger.query(&AuditFilter::default()).is_err());
    drop(ledger.enter_operation().unwrap());
    assert!(ledger.quiesce_operations(deadline()).is_err());
    mode(&root, 0o700);

    let away = root.with_extension("away");
    std::fs::rename(&root, &away).unwrap();
    assert!(ledger.query(&AuditFilter::default()).is_err());
    drop(ledger.enter_operation().unwrap());
    assert!(ledger.quiesce_operations(deadline()).is_err());
    std::fs::rename(&away, &root).unwrap();

    assert!(ledger.query(&AuditFilter::default()).is_ok());
    let _frozen = ledger.quiesce_operations(deadline()).unwrap();
}

/// 中断不是绕过冻结的门：别的实例已经冻结时，中断中的实例照样拿不到许可。
#[test]
fn an_outage_does_not_let_operations_past_a_freeze_held_elsewhere() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let freezer = SqliteAuditLedger::open(&root).unwrap();
    let worker = SqliteAuditLedger::open_existing(&root).unwrap();
    let frozen = freezer.quiesce_operations(deadline()).unwrap();
    let database = root.join("audit.sqlite3");
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert!(worker.enter_operation().is_err());
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
    drop(frozen);
    let _operation = worker.enter_operation().unwrap();
}

/// 被拒绝的目录里不留下准入文件；合法库在第一次打开后才有它。
#[test]
fn the_gate_file_appears_only_after_the_store_is_accepted() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    assert!(SqliteAuditLedger::open_existing(&root).is_err());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    assert!(root.join("checkpoint-operations.lock").is_file());
    drop(ledger);
}
