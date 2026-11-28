//! 一次文件操作在保留上占什么：墓碑名额、要保护的 revision。散件和打了包的同一套判法（T-249）：
//! 打包只挪记录、不改它占的东西，墓碑名额、保留字节、历史清理的保护打包前后一样。
use super::{
    FileOperationKind, FileOperationOutcome, FilePresence,
    disk::{self, Folder},
    request::Request,
};
use crate::{BrokerError, FsVaultBroker, Revision, VaultPath, records::WriterRecords};

/// 一次文件操作留下的、保留要算的几样。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Retained {
    /// 请求里的路径（改名是源）。
    path: VaultPath,
    /// 动手之前看到的 revision。
    before: Option<Revision>,
    /// 删除或改名。
    moves: bool,
    /// 占一个墓碑名额：删除或改名，没取消、没拒绝（还在做的也占）。
    occupies: bool,
    /// 改名的目标路径和落下的 revision。
    target: Option<(VaultPath, Option<Revision>)>,
    /// 恢复的墓碑号；它的 revision 用到时现读墓碑（墓碑从不打包）。
    restored: Option<String>,
}
impl Retained {
    /// `outcome` 是终态的结果，还在做的是 `None`；`target_revision` 是改名目标落下的 revision。
    pub(crate) fn of(
        request: &Request,
        outcome: Option<FileOperationOutcome>,
        target_revision: Option<Revision>,
    ) -> Self {
        let moves = matches!(
            request.frozen.kind,
            FileOperationKind::Delete { .. } | FileOperationKind::Rename { .. }
        );
        Self {
            path: request.frozen.path.clone(),
            before: match &request.observed_before {
                FilePresence::Present { revision, .. } => Some(*revision),
                FilePresence::Missing => None,
            },
            moves,
            occupies: moves
                && !matches!(
                    outcome,
                    Some(FileOperationOutcome::Cancelled | FileOperationOutcome::Rejected)
                ),
            target: match &request.frozen.kind {
                FileOperationKind::Rename { to, .. } => Some((to.clone(), target_revision)),
                _ => None,
            },
            restored: match &request.frozen.kind {
                FileOperationKind::RestoreTombstone { tombstone_id } => Some(tombstone_id.clone()),
                _ => None,
            },
        }
    }
    /// 占一个墓碑名额。
    pub(crate) fn occupies(&self) -> bool {
        self.occupies
    }
    /// 改名的目标路径。
    #[cfg(test)]
    pub(crate) fn target(&self) -> Option<&VaultPath> {
        self.target.as_ref().map(|(path, _)| path)
    }
    /// 整个库里要保护的 revision：保留字节按它们算。
    pub(crate) fn revisions(&self, broker: &FsVaultBroker) -> Result<Vec<Revision>, BrokerError> {
        let mut revisions = Vec::new();
        if self.moves {
            revisions.extend(self.before);
        }
        if let Some((_, revision)) = &self.target {
            revisions.extend(*revision);
        }
        if let Some(id) = &self.restored {
            revisions.push(super::tombstone::load(broker, id)?.entry()?.revision);
        }
        Ok(revisions)
    }
    /// 这条路径上要保护的 revision：历史清理按它们留。
    pub(crate) fn protects(
        &self,
        broker: &FsVaultBroker,
        path: &VaultPath,
    ) -> Result<Vec<Revision>, BrokerError> {
        let mut revisions = Vec::new();
        if let Some((to, revision)) = &self.target
            && to == path
        {
            revisions.extend(*revision);
        }
        if &self.path == path {
            revisions.extend(self.before);
            if let Some(id) = &self.restored {
                revisions.push(super::tombstone::load(broker, id)?.entry()?.revision);
            }
        }
        Ok(revisions)
    }
}

/// 一条散件请求现在占什么；改名还没终态的，目标 revision 看活动槽。
fn loose(broker: &FsVaultBroker, id: &str) -> Result<Retained, BrokerError> {
    let request = super::request::load(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
    let terminal = super::terminal::load(broker, &request.frozen)?;
    let target_revision = match (&request.frozen.kind, &terminal) {
        (FileOperationKind::Rename { .. }, Some(terminal)) => {
            terminal.rename.as_ref().and_then(|r| r.target_revision)
        }
        (FileOperationKind::Rename { .. }, None) => {
            let state = crate::mutation::load(broker)?;
            let active = state
                .file_operation
                .as_ref()
                .filter(|a| a.frozen == request.frozen)
                .ok_or(BrokerError::CorruptWriterState)?;
            active.rename.as_ref().and_then(|r| r.target_revision)
        }
        _ => None,
    };
    Ok(Retained::of(
        &request,
        terminal.map(|t| t.receipt.outcome),
        target_revision,
    ))
}

/// 全部文件操作：散件加打了包的。
pub(crate) fn all(broker: &FsVaultBroker) -> Result<Vec<Retained>, BrokerError> {
    let mut all = Vec::new();
    for name in disk::names(broker, Folder::Records)? {
        all.push(loose(broker, name.trim_end_matches(".json"))?);
    }
    all.extend(broker.archive()?.file_records().iter().cloned());
    Ok(all)
}
