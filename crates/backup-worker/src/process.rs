//! 无 shell、空环境和独占进程组；WNOWAIT 保留 leader 身份直到整组清理结束。
mod child;
mod input;
mod pipe;
mod sink;
pub(crate) mod tar;
use crate::{ProcessError, ResticCredentials, ResticPackage, Result};
use child::OwnedChild;
use pipe::Capture;
use sha2::Digest;
use sink::{Buffered, Discarded, Hashed, Sink};
use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub(crate) use tar::TreeExpectation;
use zeroize::Zeroizing;

/// JSON 输出的上限。
const STDOUT_LIMIT: usize = 8 * 1024 * 1024;
/// 普通调用 stderr 的上限；内容不留，只界定输出量。
const STDERR_LIMIT: u64 = 64 * 1024;
/// 仓库抽查 stderr 的上限：仓库坏得多时 restic 每处错误打一行，64 KiB 不够；超了算输出越界。
const CHECK_STDERR_LIMIT: u64 = 4 * 1024 * 1024;

pub(crate) struct Output {
    pub success: bool,
    pub stdout: Zeroizing<Vec<u8>>,
}
/// 不需要中途让路的调用。
fn uninterrupted() -> Result<()> {
    Ok(())
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
        Streams {
            sink: Buffered::new(STDOUT_LIMIT),
            input: None,
            stderr_limit: STDERR_LIMIT,
            interrupt: &uninterrupted,
        },
    )?;
    Ok(Output {
        success,
        stdout: stdout.bytes,
    })
}
pub(crate) fn run_input(
    package: &ResticPackage,
    argv: &[String],
    credentials: &ResticCredentials,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    bytes: &[u8],
) -> Result<Output> {
    let (success, stdout, _) = execute_with(
        package,
        argv,
        Some(credentials),
        temporary,
        deadline,
        check_source_time,
        Streams {
            sink: Buffered::new(STDOUT_LIMIT),
            input: Some(bytes),
            stderr_limit: STDERR_LIMIT,
            interrupt: &uninterrupted,
        },
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
    if expected.bytes > FILE_LIMIT || !digest_text(expected.sha256) {
        return Err(ProcessError::Configuration);
    }
    let (success, stdout, bytes) = execute_with(
        package,
        argv,
        Some(credentials),
        temporary,
        deadline,
        check_source_time,
        Streams {
            sink: Hashed::new(expected.bytes),
            input: None,
            stderr_limit: STDERR_LIMIT,
            interrupt: &uninterrupted,
        },
    )?;
    let hash = hex(&stdout.digest.finalize());
    if !success || bytes != expected.bytes || hash != expected.sha256 {
        return Err(ProcessError::Protocol);
    }
    Ok(())
}
/// 单个文件的上限：单文件读回和整份封存读回都按它核期望。
const FILE_LIMIT: u64 = 512 * 1024 * 1024 * 1024;

/// SHA-256 的文本写法：64 个小写十六进制字符。
fn digest_text(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 整份封存一次读回：一个进程输出整棵树的 tar 流，边读边按清单核（见 [`tar`]）。
pub(crate) fn verify_tree(
    package: &ResticPackage,
    argv: &[String],
    credentials: &ResticCredentials,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    expected: &TreeExpectation,
) -> Result<()> {
    let (success, stdout, _) = execute_with(
        package,
        argv,
        Some(credentials),
        temporary,
        deadline,
        check_source_time,
        Streams {
            sink: tar::TarCheck::new(expected),
            input: None,
            stderr_limit: STDERR_LIMIT,
            interrupt: &uninterrupted,
        },
    )?;
    if !success {
        return Err(ProcessError::Protocol);
    }
    stdout.finish()
}
/// 仓库抽查：stderr 上限放宽到 4 MiB；`interrupt` 只在等输出的循环里问（抽查让给备份作业），
/// 跑完之后不再问，所以已经查完的结果不会因为刚好来了作业被丢掉。
pub(crate) fn run_check(
    package: &ResticPackage,
    argv: &[String],
    credentials: &ResticCredentials,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    interrupt: &dyn Fn() -> Result<()>,
) -> Result<Output> {
    let (success, stdout, _) = execute_with(
        package,
        argv,
        Some(credentials),
        temporary,
        deadline,
        check_source_time,
        Streams {
            sink: Buffered::new(STDOUT_LIMIT),
            input: None,
            stderr_limit: CHECK_STDERR_LIMIT,
            interrupt,
        },
    )?;
    Ok(Output {
        success,
        stdout: stdout.bytes,
    })
}
struct Streams<'a, S> {
    sink: S,
    input: Option<&'a [u8]>,
    /// stderr 最多收多少字节，超了算输出越界。
    stderr_limit: u64,
    /// 子进程还没退出的每一轮都问；报错就结束整组进程，返回这个错误。退出以后只剩读空管道，不再问。
    interrupt: &'a dyn Fn() -> Result<()>,
}
fn execute_with<S: Sink>(
    package: &ResticPackage,
    argv: &[String],
    credentials: Option<&ResticCredentials>,
    temporary: &Path,
    deadline: Instant,
    check_source_time: &impl Fn() -> Result<()>,
    streams: Streams<'_, S>,
) -> Result<(bool, S, u64)> {
    if streams.input.is_some_and(|bytes| {
        bytes.is_empty()
            || bytes.len() > studiolo_backup::worker::final_manifest::FINAL_MANIFEST_MAX_BYTES
    }) {
        return Err(ProcessError::Configuration);
    }
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
        .stdin(if streams.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    if let Some(credentials) = credentials {
        credentials.attach(&mut command, deadline)?;
    }
    check_source_time()?;
    let mut child = OwnedChild::spawn(command)?;
    // Command 自己保存的环境副本在启动后马上释放；不把它留到远端作业结束。
    let result = capture(&mut child, deadline, check_source_time, streams);
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
    streams: Streams<'_, S>,
) -> Result<(S, u64)> {
    let pipe = if streams.input.is_some() {
        Some(child.stdin()?)
    } else {
        None
    };
    let interrupt = streams.interrupt;
    let mut input = input::Input::new(pipe, streams.input)?;
    let mut stdout = Capture::new(child.stdout()?, streams.sink)?;
    let mut stderr = Capture::new(child.stderr()?, Discarded::new(streams.stderr_limit))?;
    loop {
        if Instant::now() >= deadline {
            return Err(ProcessError::Deadline);
        }
        check_source_time()?;
        let input_progress = input.write()?;
        let stdout_progress = stdout.read()?;
        let stderr_progress = stderr.read()?;
        // 还在跑才问要不要让路：退出以后只剩读空管道，查完的结果不会因为刚好来了作业被丢掉。
        // 退出了也先结束可能继承输出管道的子孙，防止永远等 EOF。
        if !child.exited()? {
            interrupt()?;
        } else {
            child.kill_group()?;
            if stdout.eof && stderr.eof {
                if !input.finished() {
                    return Err(ProcessError::Io);
                }
                return Ok((stdout.sink, stdout.seen));
            }
        }
        if !stdout_progress && !stderr_progress && !input_progress {
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
