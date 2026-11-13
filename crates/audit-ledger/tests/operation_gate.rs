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
