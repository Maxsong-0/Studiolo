//! 真实控制终端仅保护输入；不是 OS 隔离，也不是远端请求的本地授权凭据。

#[path = "terminal_mode.rs"]
mod mode;
#[path = "terminal_signals.rs"]
mod signals;

use rustix::{
    fs::{self, Mode, OFlags},
    process, termios,
};
use std::{
    fmt,
    fs::File,
    io::{self, Write},
};
use studiolo_auth::TotpCode;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalError {
    NotControlling,
    Busy,
    Signal,
    Io,
    Restore,
    Cancelled,
    Eof,
    InvalidCode,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotControlling => "需要同一前台控制终端的标准输入和标准错误",
            Self::Busy => "已有终端输入正在进行",
            Self::Signal => "无法安全处理终端取消信号",
            Self::Io => "终端读取失败",
            Self::Restore => "终端恢复失败，请检查终端状态",
            Self::Cancelled => "已取消验证码输入",
            Self::Eof => "验证码输入意外结束",
            Self::InvalidCode => "验证码必须恰为六位 ASCII 数字",
        })
    }
}

impl std::error::Error for TerminalError {}

/// 所有命令先调用；不改变终端属性。必须在创建其他线程之前初始化信号处理。
pub(crate) fn preflight() -> Result<(), TerminalError> {
    controlling_terminal()?;
    signals::initialize()
}

fn controlling_terminal() -> Result<File, TerminalError> {
    let input = io::stdin();
    let error = io::stderr();
    if !termios::isatty(&input) || !termios::isatty(&error) {
        return Err(TerminalError::NotControlling);
    }
    let file = File::from(
        fs::open(
            "/dev/tty",
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY,
            Mode::empty(),
        )
        .map_err(|_| TerminalError::NotControlling)?,
    );
    let session = process::getsid(None).map_err(|_| TerminalError::NotControlling)?;
    let group = process::getpgrp();
    for fd in [
        std::os::fd::AsFd::as_fd(&input),
        std::os::fd::AsFd::as_fd(&error),
        std::os::fd::AsFd::as_fd(&file),
    ] {
        if termios::tcgetsid(fd).map_err(|_| TerminalError::NotControlling)? != session
            || termios::tcgetpgrp(fd).map_err(|_| TerminalError::NotControlling)? != group
        {
            return Err(TerminalError::NotControlling);
        }
    }
    let stdin_device = fs::fstat(&input)
        .map_err(|_| TerminalError::NotControlling)?
        .st_rdev;
    let stderr_device = fs::fstat(&error)
        .map_err(|_| TerminalError::NotControlling)?
        .st_rdev;
    if stdin_device != stderr_device {
        return Err(TerminalError::NotControlling);
    }
    Ok(file)
}

/// 只从控制终端读取；拒绝管道、EOF、非 ASCII 或第七位，不回显输入。
pub(crate) fn read_code() -> Result<TotpCode, TerminalError> {
    let terminal = controlling_terminal()?;
    let cancellation = signals::Capture::begin()?;
    cancellation.check()?;
    let mut hidden = mode::Hidden::enter(terminal)?;
    let result = (|| {
        (&hidden.file)
            .write_all("请输入六位验证码（不回显）：".as_bytes())
            .map_err(|_| TerminalError::Io)?;
        let code = read_hidden(&hidden.file, &cancellation);
        let newline = (&hidden.file)
            .write_all(b"\r\n")
            .map_err(|_| TerminalError::Io);
        newline?;
        code
    })();
    let restored = hidden.restore();
    drop(hidden);
    restored?;
    cancellation.finish()?;
    result
}

fn read_hidden(file: &File, cancellation: &signals::Capture) -> Result<TotpCode, TerminalError> {
    let mut input = Input::default();
    let mut byte = Zeroizing::new([0_u8; 1]);
    loop {
        cancellation.check()?;
        if termios::tcgetpgrp(file).map_err(|_| TerminalError::Io)? != process::getpgrp() {
            return Err(TerminalError::NotControlling);
        }
        // VMIN=0/VTIME=1：最久约100ms返回，避免信号重启阻塞 read 后无法观察取消。
        match rustix::io::read(file, &mut *byte) {
            Ok(0) => continue,
            Ok(_) => {
                cancellation.check()?;
                if let Some(code) = input.push(byte[0])? {
                    return Ok(code);
                }
            }
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return Err(TerminalError::Io),
        }
    }
}

#[derive(Default)]
struct Input {
    digits: Zeroizing<[u8; 6]>,
    length: usize,
}

impl Input {
    fn push(&mut self, byte: u8) -> Result<Option<TotpCode>, TerminalError> {
        match byte {
            3 | 26 | 28 => Err(TerminalError::Cancelled),
            4 => Err(TerminalError::Eof),
            b'\n' | b'\r' if self.length == 6 => {
                let code =
                    std::str::from_utf8(&*self.digits).map_err(|_| TerminalError::InvalidCode)?;
                TotpCode::parse(code.to_owned())
                    .map(Some)
                    .map_err(|_| TerminalError::InvalidCode)
            }
            8 | 127 => {
                self.length = self.length.saturating_sub(1);
                self.digits[self.length] = 0;
                Ok(None)
            }
            b'0'..=b'9' if self.length < 6 => {
                self.digits[self.length] = byte;
                self.length += 1;
                Ok(None)
            }
            _ => Err(TerminalError::InvalidCode),
        }
    }
}

#[cfg(test)]
#[path = "../tests/support/terminal_tests.rs"]
mod tests;
