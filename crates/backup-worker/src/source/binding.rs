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
    check: ResticInvocation,
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
            repository.clone(),
            &ResticCommand::Backup {
                tag: JobTag::for_job(&key).map_err(|_| ProcessError::Configuration)?,
                source_root: sealed.ticket().transfer_source.root.clone(),
            },
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )
        .map_err(|_| ProcessError::Configuration)?;
        let check = ResticInvocation::plan(
            repository,
            &ResticCommand::CheckRepository,
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )
        .map_err(|_| ProcessError::Configuration)?;
        Ok(Self {
            sealed,
            key,
            backup,
            check,
        })
    }
    pub fn backup(&self) -> &ResticInvocation {
        &self.backup
    }
    pub fn check(&self) -> &ResticInvocation {
        &self.check
    }
    pub fn accepts(&self, invocation: &ResticInvocation) -> Result<bool> {
        if invocation == &self.backup {
            Ok(true)
        } else if invocation == &self.check {
            Ok(false)
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
