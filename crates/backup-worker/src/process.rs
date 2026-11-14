//! 无 shell、空环境和独占进程组；WNOWAIT 保留 leader 身份直到整组清理结束。
mod child;
mod pipe;
mod sink;
use crate::{ProcessError, ResticCredentials, ResticPackage, Result};
use child::OwnedChild;
use pipe::Capture;
use sha2::Digest;
use sink::{Buffered, Hashed, Sink};
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
    let (success, stdout, _) = execute_with(
        package,
        argv,
        credentials,
        temporary,
        deadline,
        check_source_time,
        Buffered::new(8 * 1024 * 1024),
    )?;
    Ok(Output {
        success,
        stdout: stdout.bytes,
    })
}
pub(crate) struct DigestExpectation<'a> {
    pub bytes: u64,
    pub sha256: &'a str,
}
pub(crate) fn verify_digest(
    package: &ResticPackage,
    argv: &[String],
    credentials: &ResticCredentials,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    expected: DigestExpectation<'_>,
) -> Result<()> {
    if expected.bytes > 512 * 1024 * 1024 * 1024
        || expected.sha256.len() != 64
        || !expected
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProcessError::Configuration);
    }
    let (success, stdout, bytes) = execute_with(
        package,
        argv,
        Some(credentials),
        temporary,
        deadline,
        check_source_time,
        Hashed::new(expected.bytes),
    )?;
    let hash = stdout
        .digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if !success || bytes != expected.bytes || hash != expected.sha256 {
        return Err(ProcessError::Protocol);
    }
    Ok(())
}
fn execute_with<S: Sink>(
    package: &ResticPackage,
    argv: &[String],
    credentials: Option<&ResticCredentials>,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    sink: S,
) -> Result<(bool, S, u64)> {
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
        credentials.attach(&mut command, deadline)?;
    }
    check_source_time()?;
    let mut child = OwnedChild::spawn(command)?;
    // Command 自己保存的环境副本在启动后马上释放；不把它留到远端作业结束。
    let result = capture(&mut child, deadline, check_source_time, sink);
    let cleanup = child.finish();
    // 清理失败优先于原错误：worker 不得继续下一个作业。
    let status = cleanup?;
    let (stdout, count) = result?;
    package.check()?;
    if let Some(credentials) = credentials {
        credentials.check(deadline)?;
    }
    check_source_time()?;
    Ok((status.success(), stdout, count))
}
fn capture<S: Sink>(
    child: &mut OwnedChild,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    sink: S,
) -> Result<(S, u64)> {
    let mut stdout = Capture::new(child.stdout()?, sink)?;
    let mut stderr = Capture::new(child.stderr()?, Buffered::new(64 * 1024))?;
    loop {
        if Instant::now() >= deadline {
            return Err(ProcessError::Deadline);
        }
        check_source_time()?;
        let stdout_progress = stdout.read()?;
        let stderr_progress = stderr.read()?;
        if child.exited()? {
            // 即使 leader 正常退出，也先结束可能继承输出管道的子孙，防止永远等 EOF。
            child.kill_group()?;
            if stdout.eof && stderr.eof {
                return Ok((stdout.sink, stdout.seen));
            }
        }
        if !stdout_progress && !stderr_progress {
            std::thread::sleep(Duration::from_millis(2));
        }
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
