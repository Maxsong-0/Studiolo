//! 有界非阻塞输入与 stdout/stderr 同轮推进；写满立即关闭以交付 EOF。
use crate::{ProcessError, Result};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use std::{
    io::{ErrorKind, Write},
    process::ChildStdin,
};

pub(super) struct Input<'a> {
    pipe: Option<ChildStdin>,
    remaining: &'a [u8],
}
impl<'a> Input<'a> {
    pub fn new(pipe: Option<ChildStdin>, bytes: Option<&'a [u8]>) -> Result<Self> {
        if let Some(pipe) = &pipe {
            let flags = fcntl_getfl(pipe).map_err(|_| ProcessError::Io)?;
            fcntl_setfl(pipe, flags | OFlags::NONBLOCK).map_err(|_| ProcessError::Io)?;
        }
        Ok(Self {
            pipe,
            remaining: bytes.unwrap_or_default(),
        })
    }
    pub fn write(&mut self) -> Result<bool> {
        if self.remaining.is_empty() {
            self.pipe = None;
            return Ok(false);
        }
        let pipe = self.pipe.as_mut().ok_or(ProcessError::Io)?;
        let end = self.remaining.len().min(16 * 1024);
        match pipe.write(&self.remaining[..end]) {
            Ok(0) => Err(ProcessError::Io),
            Ok(count) => {
                self.remaining = &self.remaining[count..];
                if self.remaining.is_empty() {
                    self.pipe = None;
                }
                Ok(true)
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
            {
                Ok(false)
            }
            Err(_) => Err(ProcessError::Io),
        }
    }
    pub fn finished(&self) -> bool {
        self.remaining.is_empty() && self.pipe.is_none()
    }
}
