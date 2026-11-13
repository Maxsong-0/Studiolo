use crate::{ProcessError, Result};
use rustix::{
    fd::AsFd,
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
};
use std::io::{ErrorKind, Read};
use zeroize::Zeroizing;

pub(super) struct Capture<T> {
    pipe: T,
    limit: usize,
    pub bytes: Zeroizing<Vec<u8>>,
    pub eof: bool,
}
impl<T: Read + AsFd> Capture<T> {
    pub fn new(pipe: T, limit: usize) -> Result<Self> {
        let flags = fcntl_getfl(&pipe).map_err(|_| ProcessError::Io)?;
        fcntl_setfl(&pipe, flags | OFlags::NONBLOCK).map_err(|_| ProcessError::Io)?;
        Ok(Self {
            pipe,
            limit,
            bytes: Zeroizing::new(Vec::with_capacity(limit)),
            eof: false,
        })
    }
    pub fn read(&mut self) -> Result<()> {
        let mut buffer = Zeroizing::new([0_u8; 16 * 1024]);
        match self.pipe.read(&mut buffer[..]) {
            Ok(0) => self.eof = true,
            Ok(count) => {
                if count > self.limit - self.bytes.len() {
                    return Err(ProcessError::OutputLimit);
                }
                self.bytes.extend_from_slice(&buffer[..count]);
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(_) => return Err(ProcessError::Io),
        }
        Ok(())
    }
}
