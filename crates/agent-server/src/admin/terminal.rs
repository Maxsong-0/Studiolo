//! 一次性材料只写真实前台控制终端；TTY 只保护这一次显示，既不是 OS 隔离，
//! 也不构成任何远端请求的本地授权凭据，更不阻挡同 UID 或管理员读取本进程。
//!
//! 做法与 `studiolo-auth-admin` 的终端适配器、离线维护 CLI 的控制终端确认一致：
//! 固定只打开 `/dev/tty`，用会话 id、前台进程组与三个标准流的设备号共同核验身份，
//! 写入前后各复核一次。没有降级通道——核验不过就拒绝，绝不改写到标准输出。

use rustix::{fs, process, termios};
use std::{
    fmt,
    fs::File,
    io,
    os::fd::AsFd,
    time::{Duration, Instant},
};

/// 单次显示的写入期限：终端长期不接收即判失败，不无限等待。
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// 一次显示允许的最大字节数；这条通道只用于固定的一次性材料，不做通用输出。
const WRITE_LIMIT: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TerminalError {
    /// 三个标准流与 `/dev/tty` 不是同一个前台控制终端。
    NotControlling,
    /// 控制终端写入失败、短写超期或写入期间终端身份改变。
    Write,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotControlling => {
                "需要标准输入、标准输出、标准错误同为当前前台控制终端：\
                 检测到管道、重定向、后台作业，或本进程没有控制终端（CI、systemd、nohup、setsid 等）"
            }
            Self::Write => "控制终端写入失败或超期；无法确认主人已看到这次显示",
        })
    }
}

impl std::error::Error for TerminalError {}

/// 已核验的前台控制终端；持有 `/dev/tty` 句柄，每次写入前后复核身份。
pub(super) struct ControllingTerminal {
    file: File,
    session: process::Pid,
    group: process::Pid,
    device: u64,
}

impl ControllingTerminal {
    /// 打开并核验控制终端。调用方须在产生任何秘密之前调用：核验不过时直接拒绝，
    /// 此刻既未打开私有库也未生成明文，因此拒绝路径上不存在半截状态。
    pub(super) fn open() -> Result<Self, TerminalError> {
        let file = File::from(
            fs::open(
                "/dev/tty",
                fs::OFlags::RDWR | fs::OFlags::CLOEXEC | fs::OFlags::NOCTTY | fs::OFlags::NONBLOCK,
                fs::Mode::empty(),
            )
            .map_err(|_| TerminalError::NotControlling)?,
        );
        let input = io::stdin();
        let terminal = Self {
            file,
            session: process::getsid(None).map_err(|_| TerminalError::NotControlling)?,
            group: process::getpgrp(),
            device: fs::fstat(&input)
                .map_err(|_| TerminalError::NotControlling)?
                .st_rdev as u64,
        };
        terminal.revalidate()?;
        Ok(terminal)
    }

    /// 复核：会话、前台进程组、三个标准流与 `/dev/tty` 必须仍是同一个终端。
    pub(super) fn revalidate(&self) -> Result<(), TerminalError> {
        let input = io::stdin();
        let output = io::stdout();
        let error = io::stderr();
        let session = process::getsid(None).map_err(|_| TerminalError::NotControlling)?;
        let group = process::getpgrp();
        if session != self.session || group != self.group {
            return Err(TerminalError::NotControlling);
        }
        for fd in [
            input.as_fd(),
            output.as_fd(),
            error.as_fd(),
            self.file.as_fd(),
        ] {
            if !termios::isatty(fd)
                || termios::tcgetsid(fd).map_err(|_| TerminalError::NotControlling)? != session
                || termios::tcgetpgrp(fd).map_err(|_| TerminalError::NotControlling)? != group
            {
                return Err(TerminalError::NotControlling);
            }
        }
        // /dev/tty 是控制终端别名，其设备号不等于实际 PTY；同一终端以会话/前台组
        // 及固定 stdin/stdout/stderr 设备共同核验，不能把合法终端误拒为不匹配。
        for fd in [input.as_fd(), output.as_fd(), error.as_fd()] {
            if fs::fstat(fd)
                .map_err(|_| TerminalError::NotControlling)?
                .st_rdev as u64
                != self.device
            {
                return Err(TerminalError::NotControlling);
            }
        }
        Ok(())
    }

    /// 把一次性材料写进控制终端本身。即使标准输出被重定向也不会落盘，
    /// 但本命令另有更严的门禁：标准输出不是同一终端时根本不会走到这里。
    pub(super) fn show(&self, text: &str) -> Result<(), TerminalError> {
        if text.len() > WRITE_LIMIT {
            return Err(TerminalError::Write);
        }
        self.write_terminal(text.as_bytes())
    }

    fn write_terminal(&self, mut bytes: &[u8]) -> Result<(), TerminalError> {
        let deadline = Instant::now() + WRITE_TIMEOUT;
        while !bytes.is_empty() {
            self.revalidate().map_err(|_| TerminalError::Write)?;
            if Instant::now() >= deadline {
                return Err(TerminalError::Write);
            }
            match rustix::io::write(&self.file, bytes) {
                Ok(0) => return Err(TerminalError::Write),
                Ok(count) => bytes = &bytes[count..],
                // macOS 的 poll 不支持 /dev/tty；独立非阻塞描述符保持绝对截止时间。
                Err(rustix::io::Errno::AGAIN) => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => return Err(TerminalError::Write),
            }
        }
        self.revalidate().map_err(|_| TerminalError::Write)
    }
}
