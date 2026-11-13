//! 一次读取每个实际文件；全量复扫源与过滤后的独立副本后才允许发布。
use super::{MAX_BYTES, MAX_NODES, Result, SourceCatalog, StagingError, check_deadline};
use super::{
    catalog::CatalogEntry,
    io::{Directory, Regular},
};
use crate::checkpoint::inclusion::Disposition;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
use studiolo_core::{RevisionHasher, VaultPath, checkpoint::InstantMillis};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileProof {
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct TreeProof {
    pub directories: BTreeSet<String>,
    pub files: BTreeMap<VaultPath, FileProof>,
}
pub(super) struct Materialized {
    pub source: TreeProof,
    pub view: TreeProof,
    pub catalog: Vec<CatalogEntry>,
}
pub(super) fn materialize(
    source: &Directory,
    target: Option<&Directory>,
    catalog: &SourceCatalog,
    now: InstantMillis,
    deadline: Instant,
) -> Result<Materialized> {
    let mut walk = Walk {
        catalog: Some(catalog),
        now,
        deadline,
        nodes: 0,
        bytes: 0,
        source: TreeProof::default(),
        view: TreeProof::default(),
        entries: Vec::new(),
        buffer: vec![0; 256 * 1024],
        summaries: None,
    };
    if catalog.0.values().any(|r| r.redacted.is_some()) {
        walk.summaries = target.map(|t| t.create_dir("summaries")).transpose()?;
        walk.view.directories.insert("summaries".into());
    }
    walk.directory(source, target, "", 0)?;
    if let Some(summaries) = walk.summaries.as_ref() {
        summaries.readonly()?;
    }
    if walk.entries.len() != catalog.0.len() {
        return Err(StagingError::Catalog);
    }
    Ok(Materialized {
        source: walk.source,
        view: walk.view,
        catalog: walk.entries,
    })
}
pub(super) fn verify(directory: &Directory, deadline: Instant) -> Result<TreeProof> {
    let mut walk = Walk {
        catalog: None,
        now: InstantMillis::from_millis(0),
        deadline,
        nodes: 0,
        bytes: 0,
        source: TreeProof::default(),
        view: TreeProof::default(),
        entries: Vec::new(),
        buffer: vec![0; 256 * 1024],
        summaries: None,
    };
    walk.directory(directory, None, "", 0)?;
    Ok(walk.source)
}
struct Walk<'a> {
    catalog: Option<&'a SourceCatalog>,
    now: InstantMillis,
    deadline: Instant,
    nodes: u64,
    bytes: u64,
    source: TreeProof,
    view: TreeProof,
    entries: Vec<CatalogEntry>,
    buffer: Vec<u8>,
    summaries: Option<Directory>,
}
impl Walk<'_> {
    fn directory(
        &mut self,
        source: &Directory,
        target: Option<&Directory>,
        prefix: &str,
        depth: usize,
    ) -> Result<()> {
        check_deadline(self.deadline)?;
        if depth > 64 {
            return Err(StagingError::Limit);
        }
        if self.catalog.is_none() {
            source.require_readonly()?;
        }
        let names = source.names(MAX_NODES - self.nodes)?;
        self.nodes += names.len() as u64;
        for name in &names {
            check_deadline(self.deadline)?;
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            // 这是规范强制排除的版本缓存，不是用文件名替缺失的来源标签。
            if self.catalog.is_some() && path == "canonical/.stversions" {
                continue;
            }
            if source.is_directory(name)? {
                self.source.directories.insert(path.clone());
                self.view.directories.insert(path.clone());
                let child = source.child(name)?;
                let output = target.map(|dir| dir.create_dir(name)).transpose()?;
                self.directory(&child, output.as_ref(), &path, depth + 1)?;
                child.check_child(source, name)?;
            } else {
                self.file(
                    source,
                    target,
                    name,
                    VaultPath::parse(path).map_err(|_| StagingError::Catalog)?,
                )?;
            }
        }
        if source.names(MAX_NODES)? != names {
            return Err(StagingError::Changed);
        }
        if let Some(target) = target {
            target.readonly()?;
        }
        check_deadline(self.deadline)
    }
    fn file(
        &mut self,
        parent: &Directory,
        target: Option<&Directory>,
        name: &str,
        path: VaultPath,
    ) -> Result<()> {
        let registration = self
            .catalog
            .map(|catalog| catalog.0.get(&path).ok_or(StagingError::Catalog))
            .transpose()?;
        let mut revision = registration.map(|r| RevisionHasher::new(r.vault_uuid, &r.source.path));
        let mut source = Regular::open(parent, name)?;
        if self.catalog.is_none() {
            source.require_readonly()?;
        }
        if source.len() > MAX_BYTES - self.bytes {
            return Err(StagingError::Limit);
        }
        // 先算真实 revision，再按已验证范围复制；不会先把 raw 写进过滤视图。
        let proof = self.hash_file(&mut source, revision.as_mut())?;
        source.check(parent, name)?;
        self.bytes += proof.bytes;
        self.source.files.insert(path.clone(), proof.clone());
        if let Some(catalog) = self.catalog {
            let entry = catalog.inspect(
                &path,
                revision.ok_or(StagingError::Catalog)?.finish(),
                self.now,
            )?;
            match entry.disposition {
                Disposition::FullObject => {
                    if let Some(target) = target {
                        self.copy(parent, target, name, &proof)?;
                    }
                    self.view.files.insert(path.clone(), proof);
                }
                Disposition::RedactedSummary => {
                    let summary = registration
                        .and_then(|r| r.redacted.as_ref())
                        .ok_or(StagingError::Catalog)?;
                    let summary_path = entry.summary_path.as_ref().ok_or(StagingError::Catalog)?;
                    if let Some(target) = &self.summaries {
                        let name = summary_path
                            .as_str()
                            .strip_prefix("summaries/")
                            .ok_or(StagingError::Catalog)?;
                        let mut output = Regular::create(target, name, summary.bytes.len() as u64)?;
                        output.write(&summary.bytes)?;
                        output.finish()?;
                    }
                    self.view.files.insert(
                        summary_path.clone(),
                        FileProof {
                            sha256: summary.sha256.clone(),
                            bytes: summary.bytes.len() as u64,
                        },
                    );
                }
                Disposition::NonContentTombstone => {}
            }
            self.entries.push(entry);
        }
        Ok(())
    }
    fn hash_file(
        &mut self,
        file: &mut Regular,
        mut revision: Option<&mut RevisionHasher>,
    ) -> Result<FileProof> {
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        loop {
            check_deadline(self.deadline)?;
            let count = file.read(&mut self.buffer)?;
            if count == 0 {
                break;
            }
            size = size.checked_add(count as u64).ok_or(StagingError::Limit)?;
            if size > file.len() {
                return Err(StagingError::Changed);
            }
            hasher.update(&self.buffer[..count]);
            if let Some(revision) = revision.as_mut() {
                revision.update(&self.buffer[..count]);
            }
        }
        if size != file.len() {
            return Err(StagingError::Changed);
        }
        Ok(FileProof {
            sha256: super::hex(&hasher.finalize()),
            bytes: size,
        })
    }
    fn copy(
        &mut self,
        source: &Directory,
        target: &Directory,
        name: &str,
        expected: &FileProof,
    ) -> Result<()> {
        let mut input = Regular::open(source, name)?;
        let mut output = Regular::create(target, name, expected.bytes)?;
        let mut hash = Sha256::new();
        let mut size = 0_u64;
        loop {
            check_deadline(self.deadline)?;
            let count = input.read(&mut self.buffer)?;
            if count == 0 {
                break;
            }
            size = size.checked_add(count as u64).ok_or(StagingError::Limit)?;
            if size > expected.bytes {
                return Err(StagingError::Changed);
            }
            hash.update(&self.buffer[..count]);
            output.write(&self.buffer[..count])?;
        }
        input.check(source, name)?;
        if size != expected.bytes || super::hex(&hash.finalize()) != expected.sha256 {
            return Err(StagingError::Changed);
        }
        output.finish()
    }
}
