//! Startup cleanup is based only on durable, validated attempt-specific scope references.
use super::RunnerConfig;
use std::{
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use studiolo_control_db::ControlDb;
use studiolo_runner_adapter::RunnerRef;

/// Isolates every persisted real Runner before BoardPlane startup reconciliation.
/// This must run after ControlDb opens and before any claim, relay or lease is enabled.
pub fn isolate_persisted(
    control: &ControlDb,
    config: &RunnerConfig,
    forbidden: &[std::path::PathBuf],
) -> Result<(), String> {
    super::validate_isolated_roots(config, forbidden)?;
    for board in control.board_tasks().map_err(|_| "旧执行体列表不可用")? {
        let Some(task) = control.task(board.task_id).map_err(|_| "旧任务不可用")? else {
            continue;
        };
        let Some(attempt) = task.active_attempt else {
            continue;
        };
        let a = control
            .attempt(board.task_id, attempt)
            .map_err(|_| "旧attempt不可用")?
            .ok_or("旧attempt缺失")?;
        let Some(reference) = a.runner_ref_json else {
            continue;
        };
        let reference: RunnerRef =
            serde_json::from_value(reference).map_err(|_| "旧执行体引用无效")?;
        if !matches!(reference.adapter.as_str(), "claude" | "codex")
            || reference.pid.is_some()
            || reference.start_time_ticks.is_some()
            || reference.opaque_id
                != format!("studiolo-runner-{}-{}.scope", a.attempt_id, a.fence.get())
        {
            return Err("旧执行体引用无法安全隔离".into());
        }
        isolate_unit(&reference.opaque_id)?;
    }
    Ok(())
}
pub(super) fn isolate_unit(unit: &str) -> Result<(), String> {
    let _ = systemctl(&[
        "--user",
        "kill",
        "--kill-whom=all",
        "--signal=SIGKILL",
        unit,
    ]);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = systemctl(&[
            "--user",
            "show",
            "--property=LoadState,ActiveState,SubState,ControlGroup",
            "--value",
            unit,
        ])?;
        if result.status.success() {
            let state = String::from_utf8_lossy(&result.stdout);
            if state.lines().any(|s| s == "not-found") {
                return Ok(());
            }
            if let Some(group) = state
                .lines()
                .find(|s| s.starts_with('/') && !s.contains(".."))
            {
                let events =
                    std::fs::read_to_string(format!("/sys/fs/cgroup{group}/cgroup.events"))
                        .map_err(|_| "执行体cgroup状态不可用")?;
                if events.lines().any(|line| line == "populated 0") {
                    return Ok(());
                }
            } else if state.lines().any(|s| s == "inactive") {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err("执行体cgroup隔离未确认".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn systemctl(args: &[&str]) -> Result<std::process::Output, String> {
    use std::io::Read;
    let mut child = Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "systemctl不可用")?;
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = Vec::new();
                child
                    .stdout
                    .take()
                    .ok_or("systemctl输出不可用")?
                    .take(4097)
                    .read_to_end(&mut stdout)
                    .map_err(|_| "systemctl输出不可用")?;
                if stdout.len() > 4096 {
                    return Err("systemctl输出越界".into());
                }
                return Ok(std::process::Output {
                    status,
                    stdout,
                    stderr: vec![],
                });
            }
            Ok(None) if Instant::now() < until => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("systemctl超时或失败".into());
            }
        }
    }
}
