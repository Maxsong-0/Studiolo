//! 根目录最后才对组开放，完整内容复验发生在 worker 仍无法进入的 0500 根下。
use super::{Access, Directory, Mode, Regular, error, fs};
use crate::checkpoint::staging::{MAX_NODES, ReaderIdentity, Result, StagingError, check_deadline};
use std::time::Instant;

impl Directory {
    pub fn share_contents(&mut self, identity: ReaderIdentity, deadline: Instant) -> Result<()> {
        let mut remaining = MAX_NODES;
        self.share(identity, deadline, &mut remaining, 0)
    }
    fn share(
        &mut self,
        identity: ReaderIdentity,
        deadline: Instant,
        remaining: &mut u64,
        depth: usize,
    ) -> Result<()> {
        check_deadline(deadline)?;
        if self.access != Access::Private
            || depth > 64
            || identity.producer_uid != rustix::process::geteuid().as_raw()
        {
            return Err(StagingError::UnsafeRoot);
        }
        self.require_readonly()?;
        let names = self.names(*remaining)?;
        *remaining -= names.len() as u64;
        for name in &names {
            check_deadline(deadline)?;
            if self.is_directory(name)? {
                let mut child = self.child(name)?;
                child.share(identity, deadline, remaining, depth + 1)?;
                child.activate_reader()?;
                if self.child_with(name, Access::Shared(identity))?.pin != child.pin {
                    return Err(StagingError::Changed);
                }
            } else {
                let file = Regular::open(self, name)?;
                file.check(self, name)?;
                file.grant_reader(identity.reader_gid)?;
            }
        }
        if self.names(MAX_NODES)? != names {
            return Err(StagingError::Changed);
        }
        fs::fchown(
            &self.fd,
            None,
            Some(rustix::process::Gid::from_raw(identity.reader_gid)),
        )
        .map_err(error)?;
        self.sync()?;
        self.access = Access::SharingRoot(identity);
        self.require_readonly()?;
        check_deadline(deadline)
    }
    pub fn activate_reader(&mut self) -> Result<()> {
        let Access::SharingRoot(identity) = self.access else {
            return Err(StagingError::UnsafeRoot);
        };
        self.require_readonly()?;
        fs::fchmod(&self.fd, Mode::RUSR | Mode::XUSR | Mode::RGRP | Mode::XGRP).map_err(error)?;
        self.sync()?;
        self.access = Access::Shared(identity);
        self.require_readonly()
    }
}
