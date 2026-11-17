//! 配置批准的路径、同一作业引用与真实保留期共同约束每次执行。
mod binding;
mod parents;
pub(crate) mod window;
use crate::{ProcessError, Result, WorkerConfig};
use binding::Binding;
use parents::Parents;
use std::{path::PathBuf, time::Instant};
use studiolo_backup::{
    checkpoint::staging::{ReaderIdentity, SealedCheckpoint, SealedReference, StagingError},
    worker::{restic::ResticInvocation, state::JobKey},
};
use window::{Window, utc_now};

pub(crate) struct Source {
    parent: PathBuf,
    identity: ReaderIdentity,
    producer: u32,
    group: u32,
    parents: Parents,
    binding: Binding,
    window: Window,
}
impl Source {
    pub fn producer(&self) -> u32 {
        self.producer
    }
    pub fn group(&self) -> u32 {
        self.group
    }
    pub fn current_time(&self) -> Result<studiolo_core::checkpoint::InstantMillis> {
        self.window.current_time()
    }
    pub fn open(
        config: &WorkerConfig,
        key: JobKey,
        reference: &SealedReference,
        deadline: Instant,
    ) -> Result<Self> {
        if key.checkpoint_id != reference.checkpoint_id {
            return Err(ProcessError::Source);
        }
        let started = Instant::now();
        let root = SealedCheckpoint::root_path(config.sealed_parent(), &reference.checkpoint_id);
        let parents = Parents::read(&root, config.producer_uid(), config.reader_gid(), deadline)?;
        let now = utc_now()?;
        let sealed = SealedCheckpoint::open_for_worker(
            config.sealed_parent(),
            reference,
            config.reader_identity(),
            now,
            deadline,
        )
        .map_err(map_staging)?;
        let window = Window::new(
            deadline,
            started,
            now,
            sealed.ticket().content_cutoff,
            sealed.ticket().transfer_source.expires_at,
        )?;
        let source = Self {
            parent: config.sealed_parent().to_owned(),
            identity: config.reader_identity(),
            producer: config.producer_uid(),
            group: config.reader_gid(),
            parents,
            binding: Binding::new(config.repository().clone(), key, reference, sealed)?,
            window,
        };
        source.check_time()?;
        source.check_parents()?;
        source.check_time()?;
        Ok(source)
    }
    pub fn accepts(&self, invocation: &ResticInvocation) -> Result<()> {
        self.binding.accepts(invocation)
    }
    pub fn repository(&self) -> &studiolo_backup::worker::restic::RepositoryRef {
        self.binding.repository()
    }
    pub fn deadline(&self) -> Instant {
        self.window.deadline()
    }
    pub fn key(&self) -> &JobKey {
        &self.binding.key
    }
    pub fn sealed(&self) -> &SealedCheckpoint {
        &self.binding.sealed
    }
    pub fn check_time(&self) -> Result<()> {
        self.window.check()
    }
    pub fn check(&self) -> Result<()> {
        self.check_time()?;
        self.check_parents()?;
        let reopened = SealedCheckpoint::open_for_worker(
            &self.parent,
            self.sealed().reference(),
            self.identity,
            utc_now()?,
            self.deadline(),
        )
        .map_err(map_staging)?;
        self.binding.check_reopened(&reopened)?;
        self.check_parents()?;
        self.check_time()
    }
    fn check_parents(&self) -> Result<()> {
        if Parents::read(
            &self.sealed().ticket().transfer_source.root,
            self.producer,
            self.group,
            self.deadline(),
        )? != self.parents
        {
            return Err(ProcessError::Source);
        }
        Ok(())
    }
}
fn map_staging(error: StagingError) -> ProcessError {
    if error == StagingError::Expired {
        ProcessError::Deadline
    } else {
        ProcessError::Source
    }
}
