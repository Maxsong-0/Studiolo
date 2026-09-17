//! 只整理无归属 stage；先完整有界验证，未知安装或任何异常文件均不动。

use super::observation;
use crate::writer::{directory, disk};
use crate::{BrokerError, FsVaultBroker, StagePruneReport, WriterPhase};
use std::path::Path;

impl FsVaultBroker {
    /// 在唯一 owner 内清理自有孤立 stage；保留 Prepared 的 stage 与全部协调证据。
    /// 中途 I/O 故障可能已删除部分已验证孤立文件；返回错误不表示全部成功，可重新检查。
    pub fn prune_stages(&mut self) -> Result<StagePruneReport, BrokerError> {
        crate::maintenance::ensure_idle(self)?;
        let status = self.write_status()?;
        if !matches!(status.phase, WriterPhase::Ready | WriterPhase::Prepared) {
            return Err(BrokerError::MutationUnresolved);
        }
        let state = crate::mutation::load(self)?;
        if let Some(op) = &state.operation {
            crate::mutation::install::verify_stage(self, op)?;
        }
        let journal = observation::capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        let dir = directory(&self.root, ".workbench/staging")?;
        let names = stage_names(&dir)?;
        let mut files = Vec::new();
        let mut total = 0usize;
        let owned = state.operation.as_ref().map(|op| op.stage.name.as_str());
        for name in &names {
            let observed = observation::capture(&dir, name, crate::blob::OBJECT_LIMIT as usize)?;
            if observed.identity.is_none() {
                return Err(BrokerError::CaptureChanged);
            }
            total = total
                .checked_add(
                    observed
                        .identity
                        .as_ref()
                        .map_or(0, |value| value.bytes as usize),
                )
                .ok_or(BrokerError::WriterLimitExceeded)?;
            if total > crate::blob::STAGE_BYTES as usize {
                return Err(BrokerError::WriterLimitExceeded);
            }
            if Some(name.as_str()) == owned
                && let Some(op) = &state.operation
            {
                crate::mutation::install::verify_stage(self, op)?;
            }
            files.push((name, observed));
        }
        if stage_names(&dir)? != names
            || observation::capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)? != journal
        {
            return Err(BrokerError::CaptureChanged);
        }
        // 所有输入先验完成以后，才开始删除；每项再次验证身份、ctime 和字节。
        for (name, observed) in &files {
            let current = observation::capture(&dir, name, crate::blob::OBJECT_LIMIT as usize)?;
            if !current.same_file(observed) {
                return Err(BrokerError::CaptureChanged);
            }
        }
        let mut removed = 0;
        for (name, observed) in files {
            if Some(name.as_str()) == owned {
                continue;
            }
            self.writer.check(&self.root, &self.data)?;
            if observation::capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)? != journal
                || !observation::capture(&dir, name, crate::blob::OBJECT_LIMIT as usize)?
                    .same_file(&observed)
            {
                return Err(BrokerError::CaptureChanged);
            }
            dir.remove_file(name)
                .map_err(|e| BrokerError::io("prune orphan stage", e))?;
            FsVaultBroker::sync_directory(&dir, Path::new("."))?;
            removed += 1;
        }
        self.writer.check(&self.root, &self.data)?;
        if observation::capture(&self.data, "writer-state.json", disk::RECORD_LIMIT)? != journal {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(StagePruneReport {
            removed,
            retained: usize::from(owned.is_some()),
        })
    }
}

fn stage_names(dir: &cap_std::fs::Dir) -> Result<Vec<String>, BrokerError> {
    let mut names = Vec::new();
    for entry in dir
        .entries()
        .map_err(|e| BrokerError::io("list mutation stages", e))?
    {
        if names.len() >= 64 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect mutation stage", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !disk::hex(name.strip_suffix(".stage").unwrap_or("")) {
            return Err(BrokerError::CorruptWriterState);
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}
