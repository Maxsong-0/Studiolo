//! 只接受从实际封存和原作业键派生的完整调用向量。
use crate::{ProcessError, Result};
use studiolo_backup::{
    checkpoint::staging::{SealedCheckpoint, SealedReference},
    worker::{
        restic::{
            CredentialHolder, JobTag, RepositoryRef, ResticCommand, ResticIdentity,
            ResticInvocation,
        },
        state::JobKey,
    },
};

pub(super) struct Binding {
    pub sealed: SealedCheckpoint,
    pub key: JobKey,
    backup: ResticInvocation,
}
impl Binding {
    pub fn repository(&self) -> &RepositoryRef {
        self.backup.repository()
    }
    pub fn new(
        repository: RepositoryRef,
        key: JobKey,
        reference: &SealedReference,
        sealed: SealedCheckpoint,
    ) -> Result<Self> {
        if key.checkpoint_id != reference.checkpoint_id || sealed.reference() != reference {
            return Err(ProcessError::Source);
        }
        let backup = ResticInvocation::plan(
            repository,
            &ResticCommand::Backup {
                tag: JobTag::for_job(&key).map_err(|_| ProcessError::Configuration)?,
                source_root: sealed.ticket().transfer_source.root.clone(),
            },
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )
        .map_err(|_| ProcessError::Configuration)?;
        Ok(Self {
            sealed,
            key,
            backup,
        })
    }
    /// 只认从本作业派生的那一条备份调用；别的调用（含仓库抽查）一律拒绝，抽查另有执行器。
    pub fn accepts(&self, invocation: &ResticInvocation) -> Result<()> {
        if invocation == &self.backup {
            Ok(())
        } else {
            Err(ProcessError::Configuration)
        }
    }
    pub fn check_reopened(&self, reopened: &SealedCheckpoint) -> Result<()> {
        if reopened.reference() != self.sealed.reference()
            || reopened.ticket() != self.sealed.ticket()
            || reopened.manifest() != self.sealed.manifest()
            || reopened.inventory() != self.sealed.inventory()
        {
            return Err(ProcessError::Source);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/support/source_binding.rs"]
mod tests;
