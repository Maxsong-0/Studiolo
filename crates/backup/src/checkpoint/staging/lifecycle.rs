//! 原身份先于正文持久；交接完成后留下期限回执，失败只回收本次原目录。
use super::{
    ReaderIdentity, Result, SealedCheckpoint, StagingError,
    io::{Directory, JournalFile, Lease},
    journal::{self, Identity, RootIdentity},
};
use std::time::Instant;
use studiolo_core::checkpoint::CheckpointId;

pub(super) struct Lifecycle {
    lease: Lease,
    marker: Directory,
    name: String,
    identity: Identity,
    retained_name: Option<String>,
    removing_marker: bool,
    pub published: bool,
    pub finished: bool,
}
impl Lifecycle {
    pub fn begin(
        parent: &Directory,
        checkpoint_id: &CheckpointId,
        reader: Option<ReaderIdentity>,
    ) -> Result<Self> {
        let lease = parent.acquire_lease()?;
        let key = super::digest(checkpoint_id.as_str().as_bytes());
        for name in parent.names(super::MAX_NODES)? {
            if name.starts_with("incomplete-")
                || name.starts_with("export-")
                || name.starts_with("pending-")
                || name == format!("retained-{key}")
            {
                return Err(StagingError::CleanupRequired);
            }
        }
        let name = format!("incomplete-{key}");
        let marker = parent.create_dir(&name)?;
        let identity = Identity::new(parent, &marker, checkpoint_id.clone(), reader)?;
        journal::save(&marker, JournalFile::Identity, &identity)?;
        lease.check(parent)?;
        Ok(Self {
            lease,
            marker,
            name,
            identity,
            retained_name: None,
            removing_marker: false,
            published: false,
            finished: false,
        })
    }
    pub fn register(&self, parent: &Directory, kind: JournalFile, root: &Directory) -> Result<()> {
        self.check(parent)?;
        journal::save(
            &self.marker,
            kind,
            &RootIdentity {
                directory: root.identity(),
            },
        )?;
        self.check(parent)
    }
    fn current_name<'a>(&'a self, parent: &Directory) -> Result<Option<&'a str>> {
        let original = parent.exists(&self.name)?;
        let retained = self
            .retained_name
            .as_ref()
            .map(|n| parent.exists(n))
            .transpose()?
            .unwrap_or(false);
        match (original, retained) {
            (true, false) => Ok(Some(&self.name)),
            (false, true) => Ok(self.retained_name.as_deref()),
            (false, false) if self.removing_marker => Ok(None),
            _ => Err(StagingError::Changed),
        }
    }
    pub fn check(&self, parent: &Directory) -> Result<()> {
        self.lease.check(parent)?;
        let Some(name) = self.current_name(parent)? else {
            return Ok(());
        };
        self.marker.check_child(parent, name)?;
        self.marker.check_journal_layout()?;
        if journal::load::<Identity>(&self.marker, JournalFile::Identity)?.as_ref()
            != Some(&self.identity)
        {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn finish(&mut self, parent: &Directory, deadline: Instant) -> Result<()> {
        super::check_deadline(deadline)?;
        self.check(parent)?;
        self.removing_marker = true;
        self.marker
            .discard_until(parent, &self.name, None, deadline)?;
        if let Some(name) = &self.retained_name {
            self.marker.discard_until(parent, name, None, deadline)?;
        }
        self.lease.check(parent)?;
        self.finished = true;
        Ok(())
    }
    pub fn retain(
        &mut self,
        parent: &Directory,
        sealed: &SealedCheckpoint,
        deadline: Instant,
    ) -> Result<()> {
        super::check_deadline(deadline)?;
        self.check(parent)?;
        let record = self.identity.retained(sealed)?;
        journal::save(&self.marker, JournalFile::Sealed, &record)?;
        super::check_deadline(deadline)?;
        self.retained_name = Some(format!("retained-{}", self.identity.key()));
        parent.publish(
            &self.name,
            self.retained_name.as_deref().ok_or(StagingError::Changed)?,
        )?;
        self.check(parent)?;
        super::check_deadline(deadline)?;
        self.finished = true;
        Ok(())
    }
}
#[cfg(test)]
#[path = "../../../tests/private/staging_lifecycle.rs"]
mod tests;
