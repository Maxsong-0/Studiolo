use super::sink::Sink;
use crate::{ProcessError, Result};
use rustix::{
    fd::AsFd,
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};
use std::io::{ErrorKind, Read};
use zeroize::Zeroizing;

pub(super) struct Capture<T, S> {
    pipe: T,
    pub sink: S,
    pub seen: u64,
    pub eof: bool,
}
impl<T: Read + AsFd, S: Sink> Capture<T, S> {
    pub fn new(pipe: T, sink: S) -> Result<Self> {
        let flags = fcntl_getfl(&pipe).map_err(|_| ProcessError::Io)?;
        fcntl_setfl(&pipe, flags | OFlags::NONBLOCK).map_err(|_| ProcessError::Io)?;
        Ok(Self {
            pipe,
            sink,
            seen: 0,
            eof: false,
        })
    }
    pub fn read(&mut self) -> Result<bool> {
        if self.eof {
            return Ok(false);
        }
        let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
        match self.pipe.read(&mut buffer[..]) {
            Ok(0) => {
                self.eof = true;
                Ok(true)
            }
            Ok(count) => {
                if count as u64 > self.sink.limit() - self.seen {
                    return Err(ProcessError::OutputLimit);
                }
                self.sink.append(&buffer[..count]);
                self.seen += count as u64;
                Ok(true)
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {
                Ok(false)
            }
            Err(_) => Err(ProcessError::Io),
        }
    }
}
