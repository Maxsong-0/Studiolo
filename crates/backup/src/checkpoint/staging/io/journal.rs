//! 生命周期日志仅能写四个固定非正文文件；先刷临时文件，再以 NOREPLACE 发布。
use super::{Directory, Regular, Result, StagingError};

pub(in crate::checkpoint::staging) const LIMIT: usize = 16 * 1024;
#[derive(Clone, Copy)]
pub(in crate::checkpoint::staging) enum JournalFile {
    Identity,
    Export,
    Pending,
    Sealed,
}
impl JournalFile {
    pub fn names(self) -> (&'static str, &'static str) {
        match self {
            Self::Identity => ("identity.json", "identity.new"),
            Self::Export => ("export.json", "export.new"),
            Self::Pending => ("pending.json", "pending.new"),
            Self::Sealed => ("sealed.json", "sealed.new"),
        }
    }
}
impl Directory {
    pub fn check_journal_layout(&self) -> Result<()> {
        for name in self.names(8)? {
            let kind = [
                JournalFile::Identity,
                JournalFile::Export,
                JournalFile::Pending,
                JournalFile::Sealed,
            ]
            .into_iter()
            .find(|kind| {
                let (final_name, temporary) = kind.names();
                name == final_name || name == temporary
            })
            .ok_or(StagingError::Changed)?;
            if name == kind.names().0 {
                self.read_journal(kind)?.ok_or(StagingError::Changed)?;
            } else {
                // 中断只可能留下有界普通临时文件，不能把同名目录当成本程序创建的日志。
                let file = Regular::open(self, &name)?;
                if file.len() > LIMIT as u64 {
                    return Err(StagingError::Limit);
                }
                file.check(self, &name)?;
            }
        }
        Ok(())
    }
    pub fn identity(&self) -> (u64, u64) {
        self.pin
    }
    pub fn save_journal(&self, kind: JournalFile, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() || bytes.len() > LIMIT {
            return Err(StagingError::Limit);
        }
        let (name, temporary) = kind.names();
        if self.exists(name)? {
            return Err(StagingError::AlreadyExists);
        }
        let mut file = Regular::create(self, temporary, bytes.len() as u64)?;
        file.write(bytes)?;
        file.finish()?;
        self.sync()?;
        self.publish(temporary, name)?;
        if self.read_journal(kind)?.as_deref() != Some(bytes) {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn read_journal(&self, kind: JournalFile) -> Result<Option<Vec<u8>>> {
        let name = kind.names().0;
        if !self.exists(name)? {
            return Ok(None);
        }
        let mut file = Regular::open(self, name)?;
        file.require_readonly()?;
        if file.len() == 0 || file.len() > LIMIT as u64 {
            return Err(StagingError::Limit);
        }
        let mut bytes = Vec::with_capacity(file.len() as usize);
        let mut buffer = [0; 1024];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            if bytes.len() + n > file.len() as usize {
                return Err(StagingError::Changed);
            }
            bytes.extend_from_slice(&buffer[..n]);
        }
        if bytes.len() != file.len() as usize {
            return Err(StagingError::Changed);
        }
        file.check(self, name)?;
        Ok(Some(bytes))
    }
}
