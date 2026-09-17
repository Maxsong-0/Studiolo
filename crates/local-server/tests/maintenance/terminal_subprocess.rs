use super::super::{MaintenanceError, terminal::Terminal};
use std::{
    ffi::OsString,
    time::{Duration, Instant},
};
#[path = "process_support.rs"]
mod process_support;

const CHILD: &str = "STUDIOLO_MAINTENANCE_TEST_CHILD";

#[test]
fn real_pty_enforces_confirmation_deadline_and_rechecks_stdio_identity() {
    for mode in [
        "deadline",
        "wrong",
        "stdio-drift",
        "long-review",
        "oversize",
    ] {
        let args: Vec<OsString> = [
            "--exact",
            "maintenance::tests::terminal_subprocess::pty_child",
            "--nocapture",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        let mut command = process_support::script(&std::env::current_exe().unwrap(), &args);
        command.env(CHILD, mode);
        let (status, output) =
            process_support::output(command, (mode == "wrong").then_some(b"wrong\n".as_slice()));
        assert!(status.success(), "{output}");
        assert!(output.contains("MAINTENANCE_PTY_CHECKED"), "{output}");
        if mode == "long-review" {
            let value = output
                .lines()
                .find_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
                .expect("长审阅必须完整到达真实终端");
            assert_eq!(value["value"], "x".repeat(192 * 1024));
            assert_eq!(value["end"], "LONG_REVIEW_END");
        }
    }
}

#[test]
fn pty_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let terminal = Terminal::open().expect("测试子进程必须获得真实前台控制 TTY");
    if mode == "long-review" {
        terminal
            .review(&serde_json::json!({
                "value": "x".repeat(192 * 1024), "end": "LONG_REVIEW_END"
            }))
            .unwrap();
    } else if mode == "oversize" {
        assert!(matches!(
            terminal.review(&serde_json::json!({"value": "x".repeat(512 * 1024)})),
            Err(MaintenanceError::Output)
        ));
    } else if mode == "stdio-drift" {
        let null = std::fs::File::open("/dev/null").unwrap();
        rustix::stdio::dup2_stdin(&null).unwrap();
        assert!(matches!(
            terminal.revalidate(),
            Err(MaintenanceError::Terminal)
        ));
    } else {
        let started = Instant::now();
        let timeout = if mode == "deadline" {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(2)
        };
        assert!(matches!(
            terminal.confirm_short("audit-flush", timeout),
            Err(MaintenanceError::Confirmation)
        ));
        if mode == "deadline" {
            assert!(started.elapsed() >= timeout);
        } else {
            assert!(started.elapsed() < timeout);
        }
    }
    println!("MAINTENANCE_PTY_CHECKED");
}
