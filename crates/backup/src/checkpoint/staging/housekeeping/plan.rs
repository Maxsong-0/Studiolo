//! 所有根与日志先核对完再允许删除；中途已删除的原根可沿原登记继续回收。
//! 已改名成 `removing-<key>-<inode>` 的登记是上一轮删到一半停下的：名字里的 inode 要对得上，
//! 身份记录还在就整份核对，不在了目录只能已经删空；别的残缺照旧拒绝，不按名字猜。
use super::*;
use crate::checkpoint::staging::journal::{self, Identity, Retained, RootIdentity};
use std::collections::BTreeSet;

pub(super) struct Plan {
    pub name: String,
    /// 删登记目录之前先改成的名字；上一轮已经改过名的就是 `name` 本身。
    pub removing: String,
    pub marker: Directory,
    pub roots: Vec<(String, Directory)>,
    pub expires_at: Option<InstantMillis>,
    /// 上一轮删到一半、这一轮接着删的登记。
    pub resumed: bool,
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
enum Kind {
    Incomplete,
    Retained,
    /// 已改名、删到一半；带着名字里写的原 inode。
    Removing(u64),
}
/// 认登记目录的名字；前缀对、后面不是规范写法的，一律当成要人看的残留。
fn classify(name: &str) -> Result<Option<(&str, Kind)>> {
    let (key, kind) = if let Some(key) = name.strip_prefix("incomplete-") {
        (key, Kind::Incomplete)
    } else if let Some(key) = name.strip_prefix("retained-") {
        (key, Kind::Retained)
    } else if name.starts_with(journal::REMOVING) {
        let (key, inode) = journal::parse_removing(name).ok_or(StagingError::CleanupRequired)?;
        (key, Kind::Removing(inode))
    } else {
        return Ok(None);
    };
    if !studiolo_core::is_sha256_hex(key) {
        return Err(StagingError::CleanupRequired);
    }
    Ok(Some((key, kind)))
}
pub(super) fn read_all(
    parent: &Directory,
    reader: Option<ReaderIdentity>,
    now: InstantMillis,
    deadline: Instant,
) -> Result<Vec<Plan>> {
    let names = parent.names(1 + 5 * 256)?;
    let known: BTreeSet<_> = names.iter().cloned().collect();
    let scan = Scan {
        parent,
        reader,
        now,
        known: &known,
    };
    let mut covered = BTreeSet::from([".producer-lease".to_owned()]);
    let mut keys = BTreeSet::new();
    let mut plans = Vec::new();
    for name in &names {
        check_deadline(deadline)?;
        let Some((key, kind)) = classify(name)? else {
            continue;
        };
        // 同一个 key 只能有一份登记，不管它叫哪个名字。
        if plans.len() == 256 || !keys.insert(key.to_owned()) {
            return Err(StagingError::CleanupRequired);
        }
        let marker = parent.child(name)?;
        marker.check_journal_layout()?;
        let files = marker.names(8)?;
        let plan = match kind {
            Kind::Incomplete => scan.registered(name, key, false, marker, files)?,
            Kind::Retained => scan.registered(name, key, true, marker, files)?,
            Kind::Removing(inode) => scan.removing(name, key, inode, marker, files)?,
        };
        covered.extend(plan.roots.iter().map(|(root, _)| root.clone()));
        covered.insert(name.clone());
        plans.push(plan);
    }
    if known != covered {
        return Err(StagingError::CleanupRequired);
    }
    Ok(plans)
}
struct Scan<'a> {
    parent: &'a Directory,
    reader: Option<ReaderIdentity>,
    now: InstantMillis,
    known: &'a BTreeSet<String>,
}
impl Scan<'_> {
    /// 还叫原名（`incomplete-`、`retained-`）的登记：日志齐全、身份对得上，原根按登记的 inode 认。
    fn registered(
        &self,
        name: &str,
        key: &str,
        retained: bool,
        marker: Directory,
        files: Vec<String>,
    ) -> Result<Plan> {
        let Self {
            parent,
            reader,
            now,
            known,
        } = *self;
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
                roots.push((root_name, root));
            } else if prefix == "sealed" && expires_at.is_some_and(|expiry| expiry > now) {
                return Err(StagingError::Changed);
            }
        }
        Ok(Plan {
            name: name.to_owned(),
            removing: journal::removing_name(key, marker.identity()),
            marker,
            roots,
            expires_at,
            resumed: false,
            files,
            proof,
        })
    }
    /// 已改名、删到一半的登记。名字里的 inode 要对得上这个目录；身份记录最后删，它还在就整份核对，
    /// 不在了目录只能已经删空。改名之前同一个 key 的原根都已删完，所以它不认领任何原根：
    /// 这时还有同一个 key 的原根，就落在最后「每个名字都有人认领」那道检查上。
    fn removing(
        &self,
        name: &str,
        key: &str,
        inode: u64,
        marker: Directory,
        files: Vec<String>,
    ) -> Result<Plan> {
        let Self {
            parent,
            reader,
            now,
            ..
        } = *self;
        if marker.identity().1 != inode {
            return Err(StagingError::Changed);
        }
        let mut proof = Vec::new();
        let identity: Option<Identity> = load(&marker, JournalFile::Identity, &mut proof)?;
        match &identity {
            Some(identity) => identity.validate(parent, &marker, key, reader, now)?,
            None if files.is_empty() => {}
            None => return Err(StagingError::CleanupRequired),
        }
        let _: Option<RootIdentity> = load(&marker, JournalFile::Export, &mut proof)?;
        let _: Option<RootIdentity> = load(&marker, JournalFile::Pending, &mut proof)?;
        let sealed: Option<Retained> = load(&marker, JournalFile::Sealed, &mut proof)?;
        if let (Some(identity), Some(record)) = (&identity, &sealed) {
            record.validate(identity, now)?;
        }
        Ok(Plan {
            name: name.to_owned(),
            removing: name.to_owned(),
            marker,
            roots: Vec::new(),
            expires_at: None,
            resumed: true,
            files,
            proof,
        })
    }
}
