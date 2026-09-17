//! 本机 OS 身份与有界终端确认；不将配置主人或 TTY 当作应用会话认证。

use super::MaintenanceError;
use rustix::{fs, process, termios};
use std::{
    fs::File,
    io,
    os::fd::AsFd,
    time::{Duration, Instant},
};

const LIMIT: usize = 128;
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Operator {
    pub(super) uid: u32,
    pub(super) gid: u32,
}

pub(super) struct Terminal {
    file: File,
    operator: Operator,
    session: process::Pid,
    group: process::Pid,
    device: u64,
}

impl Terminal {
    pub(super) fn open() -> Result<Self, MaintenanceError> {
        let operator = actual_operator()?;
        let file = File::from(
            fs::open(
                "/dev/tty",
                fs::OFlags::RDWR | fs::OFlags::CLOEXEC | fs::OFlags::NOCTTY | fs::OFlags::NONBLOCK,
                fs::Mode::empty(),
            )
            .map_err(|_| MaintenanceError::Terminal)?,
        );
        let input = io::stdin();
        let terminal = Self {
            file,
            operator,
            session: process::getsid(None).map_err(|_| MaintenanceError::Terminal)?,
            group: process::getpgrp(),
            device: fs::fstat(&input)
                .map_err(|_| MaintenanceError::Terminal)?
                .st_rdev as u64,
        };
        terminal.revalidate()?;
        Ok(terminal)
    }

    pub(super) fn operator(&self) -> Operator {
        self.operator
    }

    pub(super) fn review(&self, value: &serde_json::Value) -> Result<(), MaintenanceError> {
        self.revalidate()?;
        // 即使 stdout 被重定向，实际确认者也必须在同一控制终端看到审阅元数据。
        let bytes = serde_json::to_vec(value).map_err(|_| MaintenanceError::Output)?;
        if bytes.len() > 512 * 1024 {
            return Err(MaintenanceError::Output);
        }
        self.write_terminal(&bytes)?;
        self.write_terminal(b"\n")
    }

    fn write_terminal(&self, mut bytes: &[u8]) -> Result<(), MaintenanceError> {
        let deadline = Instant::now() + TIMEOUT;
        while !bytes.is_empty() {
            self.revalidate()?;
            if Instant::now() >= deadline {
                return Err(MaintenanceError::Terminal);
            }
            match rustix::io::write(&self.file, bytes) {
                Ok(0) => return Err(MaintenanceError::Terminal),
                Ok(count) => bytes = &bytes[count..],
                Err(rustix::io::Errno::AGAIN) => std::thread::sleep(Duration::from_millis(25)),
                Err(_) => return Err(MaintenanceError::Terminal),
            }
        }
        self.revalidate()
    }

    pub(super) fn revalidate(&self) -> Result<(), MaintenanceError> {
        if actual_operator()? != self.operator {
            return Err(MaintenanceError::Terminal);
        }
        let input = io::stdin();
        let error = io::stderr();
        let session = process::getsid(None).map_err(|_| MaintenanceError::Terminal)?;
        let group = process::getpgrp();
        if session != self.session || group != self.group {
            return Err(MaintenanceError::Terminal);
        }
        for fd in [input.as_fd(), error.as_fd(), self.file.as_fd()] {
            if !termios::isatty(fd)
                || termios::tcgetsid(fd).map_err(|_| MaintenanceError::Terminal)? != session
                || termios::tcgetpgrp(fd).map_err(|_| MaintenanceError::Terminal)? != group
            {
                return Err(MaintenanceError::Terminal);
            }
        }
        // /dev/tty 是控制终端别名，其设备号不等于实际 PTY；同一终端以会话/前台组
        // 及固定 stdin/stderr 设备共同核验，不能把合法终端误拒为不匹配。
        for fd in [input.as_fd(), error.as_fd()] {
            if fs::fstat(fd)
                .map_err(|_| MaintenanceError::Terminal)?
                .st_rdev as u64
                != self.device
            {
                return Err(MaintenanceError::Terminal);
            }
        }
        Ok(())
    }

    pub(super) fn confirm(&self, expected: &str) -> Result<(), MaintenanceError> {
        self.confirm_with_timeout(expected, TIMEOUT)
    }

    #[cfg(test)]
    pub(super) fn confirm_short(
        &self,
        expected: &str,
        timeout: Duration,
    ) -> Result<(), MaintenanceError> {
        self.confirm_with_timeout(expected, timeout)
    }

    fn confirm_with_timeout(
        &self,
        expected: &str,
        timeout: Duration,
    ) -> Result<(), MaintenanceError> {
        self.revalidate()?;
        if expected.is_empty()
            || expected.len() >= LIMIT
            || !expected.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        {
            return Err(MaintenanceError::Confirmation);
        }
        let prompt =
            format!("\n本机维护不会证明原写入成功。请在 120 秒内完整输入：\n{expected}\n> ");
        self.write_terminal(prompt.as_bytes())?;
        let deadline = Instant::now() + timeout;
        let mut input = Input::default();
        let mut bytes = [0_u8; LIMIT + 1];
        loop {
            self.revalidate()?;
            if Instant::now() >= deadline {
                return Err(MaintenanceError::Confirmation);
            }
            match rustix::io::read(&self.file, &mut bytes) {
                Ok(0) => return Err(MaintenanceError::Confirmation),
                Ok(count) => {
                    if let Some(line) = input.push(&bytes[..count])? {
                        if Instant::now() >= deadline || line != expected {
                            return Err(MaintenanceError::Confirmation);
                        }
                        return self.revalidate();
                    }
                }
                Err(rustix::io::Errno::AGAIN) => {
                    // macOS 的 poll 不支持 /dev/tty；独立非阻塞描述符保持绝对截止时间。
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return Err(MaintenanceError::Confirmation),
            }
        }
    }
}

pub(super) fn actual_operator() -> Result<Operator, MaintenanceError> {
    let uid = process::getuid();
    let gid = process::getgid();
    validate_ids(
        uid.as_raw(),
        process::geteuid().as_raw(),
        gid.as_raw(),
        process::getegid().as_raw(),
    )
}

pub(super) fn validate_ids(
    uid: u32,
    effective_uid: u32,
    gid: u32,
    effective_gid: u32,
) -> Result<Operator, MaintenanceError> {
    if uid != effective_uid || gid != effective_gid {
        return Err(MaintenanceError::Terminal);
    }
    Ok(Operator { uid, gid })
}

#[derive(Default)]
pub(super) struct Input {
    bytes: Vec<u8>,
}

impl Input {
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Option<String>, MaintenanceError> {
        if self.bytes.len() + bytes.len() > LIMIT {
            return Err(MaintenanceError::Confirmation);
        }
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte == b'\n' && index + 1 == bytes.len() {
                return String::from_utf8(std::mem::take(&mut self.bytes))
                    .map(Some)
                    .map_err(|_| MaintenanceError::Confirmation);
            }
            if !(byte.is_ascii_graphic() || byte == b' ') {
                return Err(MaintenanceError::Confirmation);
            }
            self.bytes.push(byte);
        }
        Ok(None)
    }
}
