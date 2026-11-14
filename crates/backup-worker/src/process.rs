//! 无 shell、空环境和独占进程组；WNOWAIT 保留 leader 身份直到整组清理结束。
mod child;
mod pipe;
use crate::{ProcessError, ResticCredentials, ResticPackage, Result};
use child::OwnedChild;
use pipe::Capture;
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub(crate) struct Output {
    pub success: bool,
    pub stdout: Zeroizing<Vec<u8>>,
}
pub(crate) fn run_scoped(
    package: &ResticPackage,
    argv: &[String],
    credentials: Option<&ResticCredentials>,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
) -> Result<Output> {
    if Instant::now() >= deadline {
        return Err(ProcessError::Deadline);
    }
    check_source_time()?;
    let executable = package.executable()?;
    let mut command = Command::new(&executable);
    command
        .args(argv)
        .env_clear()
        .env("TMPDIR", temporary)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(credentials) = credentials {
        credentials.attach(&mut command);
    }
    let mut child = OwnedChild::spawn(command)?;
    // Command 自己保存的环境副本在启动后马上释放；不把它留到远端作业结束。
    let result = capture(&mut child, deadline, check_source_time);
    let cleanup = child.finish();
    // 清理失败优先于原错误：worker 不得继续下一个作业。
    let status = cleanup?;
    let stdout = result?;
    package.check()?;
    check_source_time()?;
    Ok(Output {
        success: status.success(),
        stdout,
    })
}
fn capture(
    child: &mut OwnedChild,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
) -> Result<Zeroizing<Vec<u8>>> {
    let mut stdout = Capture::new(child.stdout()?, 8 * 1024 * 1024)?;
    let mut stderr = Capture::new(child.stderr()?, 64 * 1024)?;
    loop {
        if Instant::now() >= deadline {
            return Err(ProcessError::Deadline);
        }
        check_source_time()?;
        stdout.read()?;
        stderr.read()?;
        if child.exited()? {
            // 即使 leader 正常退出，也先结束可能继承输出管道的子孙，防止永远等 EOF。
            child.kill_group()?;
            if stdout.eof && stderr.eof {
                return Ok(stdout.bytes);
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[cfg(test)]
pub(crate) fn run(
    package: &ResticPackage,
    argv: &[String],
    credentials: Option<&ResticCredentials>,
    temporary: &Path,
    deadline: Instant,
) -> Result<Output> {
    run_scoped(package, argv, credentials, temporary, deadline, &|| Ok(()))
}
