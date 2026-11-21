//! 原身份先于正文持久；交接完成后留下期限回执，失败只回收本次原目录。
//! 删登记目录先原子改名成带原 inode 的「删除中」名字，删到一半停下也能接着删完。
use super::{
    ReaderIdentity, Result, SealedCheckpoint, StagingError,
    budget::Budget,
    io::{Directory, JournalFile, Lease},
    journal::{self, Identity, RootIdentity},
};
use std::time::Instant;
use studiolo_core::checkpoint::CheckpointId;

pub(super) struct Lifecycle {
    lease: Lease,
    marker: Directory,
    name: String,
    /// 删登记目录之前先改成的名字（带它自己的 inode），见 `journal::removing_name`。
    removing: String,
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
                || name.starts_with(journal::REMOVING)
                || name == format!("retained-{key}")
            {
                return Err(StagingError::CleanupRequired);
            }
        }
        let name = format!("incomplete-{key}");
        let marker = parent.create_dir(&name)?;
        let removing = journal::removing_name(&key, marker.identity());
        let identity = Identity::new(parent, &marker, checkpoint_id.clone(), reader)?;
        journal::save(&marker, JournalFile::Identity, &identity)?;
        lease.check(parent)?;
        Ok(Self {
            lease,
            marker,
            name,
            removing,
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
        let removing = parent.exists(&self.removing)?;
        match (original, retained, removing) {
            (true, false, false) => Ok(Some(&self.name)),
            (false, true, false) => Ok(self.retained_name.as_deref()),
            // 上一次 `finish` 已经改了名、删到一半；只有已经开始删登记之后才认这个名字。
            (false, false, true) if self.removing_marker => Ok(Some(&self.removing)),
            (false, false, false) if self.removing_marker => Ok(None),
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
        let identity = journal::load::<Identity>(&self.marker, JournalFile::Identity)?;
        // 身份记录最后删：删到一半的登记里它不在了，目录就只能已经删空。
        let emptied =
            name == self.removing && identity.is_none() && self.marker.names(8)?.is_empty();
        if identity.as_ref() != Some(&self.identity) && !emptied {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    pub fn finish(&mut self, parent: &Directory, budget: &mut Budget) -> Result<()> {
        budget.check()?;
        self.check(parent)?;
        self.removing_marker = true;
        if let Some(name) = self.current_name(parent)? {
            let last = JournalFile::Identity.names().0;
            self.marker
                .retire(parent, name, &self.removing, last, budget)?;
        }
        self.lease.check(parent)?;
        self.finished = true;
        Ok(())
    }
    /// 已改名成 `retained-`、引用却没交出去（改名后父目录 fsync 失败、之后的复核失败）：失败回收删原根之前，
    /// 先把登记原子改回原名。未到期的 `retained-` 要求封存根在场，根删了、登记没删完就要等到期才收得走；
    /// 改回原名后不管停在哪一步，留下的都是普通的未完成登记，下一轮回收照常收走。
    pub fn unretain(&mut self, parent: &Directory, budget: &mut Budget) -> Result<()> {
        budget.check()?;
        self.check(parent)?;
        let Some(retained) = self.retained_name.clone() else {
            return Ok(());
        };
        if parent.exists(&retained)? {
            budget.spend()?;
            parent.publish(&retained, &self.name)?;
        }
        self.retained_name = None;
        self.check(parent)
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
#[path = "../../../tests/private/staging_retire.rs"]
mod retire_tests;
#[cfg(test)]
#[path = "../../../tests/private/staging_lifecycle.rs"]
mod tests;
