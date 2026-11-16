//! 所有根与日志先核对完再允许删除；中途已删除的原根可沿原登记继续回收。
use super::*;
use crate::checkpoint::staging::{
    io::JournalFile,
    journal::{Identity, Retained, RootIdentity},
};
use std::collections::BTreeSet;

pub(super) struct Plan {
    pub name: String,
    pub marker: Directory,
    pub roots: Vec<(String, Directory)>,
    pub expires_at: Option<InstantMillis>,
    files: Vec<String>,
    proof: Vec<(JournalFile, Option<Vec<u8>>)>,
}
impl Plan {
    pub fn check(&self, parent: &Directory) -> Result<()> {
        self.marker.check_child(parent, &self.name)?;
        self.marker.check_journal_layout()?;
        if self.marker.names(8)? != self.files {
            return Err(StagingError::Changed);
        }
        for (kind, expected) in &self.proof {
            if self.marker.read_journal(*kind)? != *expected {
                return Err(StagingError::Changed);
            }
        }
        Ok(())
    }
}
fn load<T: serde::de::DeserializeOwned>(
    marker: &Directory,
    kind: JournalFile,
    proof: &mut Vec<(JournalFile, Option<Vec<u8>>)>,
) -> Result<Option<T>> {
    let bytes = marker.read_journal(kind)?;
    let record = bytes
        .as_deref()
        .map(|value| serde_json::from_slice(value).map_err(|_| StagingError::Changed))
        .transpose()?;
    proof.push((kind, bytes));
    Ok(record)
}
pub(super) fn read_all(
    parent: &Directory,
    reader: Option<ReaderIdentity>,
    now: InstantMillis,
    deadline: Instant,
) -> Result<Vec<Plan>> {
    let names = parent.names(1 + 5 * 256)?;
    let known: BTreeSet<_> = names.iter().cloned().collect();
    let mut covered = BTreeSet::from([".producer-lease".to_owned()]);
    let mut keys = BTreeSet::new();
    let mut plans = Vec::new();
    for name in &names {
        check_deadline(deadline)?;
        let (key, retained) = if let Some(key) = name.strip_prefix("incomplete-") {
            (key, false)
        } else if let Some(key) = name.strip_prefix("retained-") {
            (key, true)
        } else {
            continue;
        };
        if plans.len() == 256
            || key.len() != 64
            || !key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !keys.insert(key.to_owned())
        {
            return Err(StagingError::CleanupRequired);
        }
        let marker = parent.child(name)?;
        marker.check_journal_layout()?;
        let files = marker.names(8)?;
        let committed = [
            "identity.json",
            "export.json",
            "pending.json",
            "sealed.json",
        ];
        let allowed = [
            "identity.json",
            "identity.new",
            "export.json",
            "export.new",
            "pending.json",
            "pending.new",
            "sealed.json",
            "sealed.new",
        ];
        if files.iter().any(|name| !allowed.contains(&name.as_str()))
            || retained
                && (files.len() != committed.len()
                    || committed.iter().any(|n| !files.iter().any(|v| v == n)))
        {
            return Err(StagingError::Changed);
        }
        let mut proof = Vec::new();
        let identity: Identity = load(&marker, JournalFile::Identity, &mut proof)?
            .ok_or(StagingError::CleanupRequired)?;
        identity.validate(parent, &marker, key, reader, now)?;
        let export: Option<RootIdentity> = load(&marker, JournalFile::Export, &mut proof)?;
        let pending: Option<RootIdentity> = load(&marker, JournalFile::Pending, &mut proof)?;
        let sealed: Option<Retained> = load(&marker, JournalFile::Sealed, &mut proof)?;
        if let Some(record) = &sealed {
            record.validate(&identity, now)?;
        }
        if retained && (export.is_none() || pending.is_none() || sealed.is_none()) {
            return Err(StagingError::Changed);
        }
        let expires_at = if retained {
            Some(sealed.as_ref().ok_or(StagingError::Changed)?.expires_at()?)
        } else {
            None
        };
        let mut roots = Vec::new();
        for (prefix, proof) in [
            ("export", export.as_ref()),
            ("pending", pending.as_ref()),
            ("sealed", pending.as_ref()),
        ] {
            let root_name = format!("{prefix}-{key}");
            if known.contains(&root_name) {
                if retained && prefix != "sealed" {
                    return Err(StagingError::Changed);
                }
                let proof = proof.ok_or(StagingError::CleanupRequired)?;
                let root = parent.reclaim_child(&root_name, proof.directory, reader)?;
                roots.push((root_name.clone(), root));
                covered.insert(root_name);
            } else if prefix == "sealed" && expires_at.is_some_and(|expiry| expiry > now) {
                return Err(StagingError::Changed);
            }
        }
        covered.insert(name.clone());
        plans.push(Plan {
            name: name.clone(),
            marker,
            roots,
            expires_at,
            files,
            proof,
        });
    }
    if known != covered {
        return Err(StagingError::CleanupRequired);
    }
    Ok(plans)
}
