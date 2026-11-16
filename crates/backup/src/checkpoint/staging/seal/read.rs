//! 用可信角色打开原路径，每次重开都重新核验全部清单和内容。
use super::super::{Directory, ReaderIdentity, Result, StagingError, check_deadline, tree};
use super::{
    DiskSeal, InclusionDocument, SealedCheckpoint, SealedInventory, SealedReference,
    expiry_deadline, read,
};
use crate::checkpoint::manifest::{CheckpointManifest, ManifestInputs};
use std::{path::Path, time::Instant};
use studiolo_core::checkpoint::InstantMillis;

impl SealedCheckpoint {
    /// 私有封存重开；引用来自已批准的请求，不能扫描目录选择最新尝试。
    pub fn open(
        parent: &Path,
        reference: &SealedReference,
        now: InstantMillis,
        deadline: Instant,
    ) -> Result<Self> {
        let directory = Directory::open(parent, true)?;
        Self::open_parent(parent, &directory, reference, now, deadline)
    }
    /// 生产者恢复自己的待交接记录；生产者身份固定为当前系统 UID。
    pub fn open_shared(
        parent: &Path,
        reference: &SealedReference,
        reader_gid: u32,
        now: InstantMillis,
        deadline: Instant,
    ) -> Result<Self> {
        let directory = Directory::reader_parent(parent, ReaderIdentity::producer(reader_gid)?)?;
        Self::open_parent(parent, &directory, reference, now, deadline)
    }
    /// 独立 worker 重开；identity 只允许来自其受保护的部署配置。
    pub fn open_for_worker(
        parent: &Path,
        reference: &SealedReference,
        identity: ReaderIdentity,
        now: InstantMillis,
        deadline: Instant,
    ) -> Result<Self> {
        identity.require_worker()?;
        let directory = Directory::reader_parent(parent, identity)?;
        Self::open_parent(parent, &directory, reference, now, deadline)
    }
    fn open_parent(
        parent: &Path,
        directory: &Directory,
        reference: &SealedReference,
        now: InstantMillis,
        deadline: Instant,
    ) -> Result<Self> {
        check_deadline(deadline)?;
        let root_path = Self::root_path(parent, &reference.checkpoint_id);
        let name = root_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(StagingError::UnsafeRoot)?;
        let root = directory.child(name)?;
        let result = Self::verify_directory(&root, &root_path, reference, now, deadline)?;
        root.check_child(directory, name)?;
        directory.check_path(parent)?;
        check_deadline(deadline)?;
        Ok(result)
    }
    pub(super) fn verify_directory(
        directory: &Directory,
        root_path: &Path,
        reference: &SealedReference,
        now: InstantMillis,
        deadline: Instant,
    ) -> Result<Self> {
        let started = Instant::now();
        check_deadline(deadline)?;
        directory.require_readonly()?;
        if directory.names(5)? != ["data", "inclusion.json", "manifest.json", "seal.json"] {
            return Err(StagingError::Manifest);
        }
        let (seal, hash, seal_bytes): (DiskSeal, _, _) = read(directory, "seal.json", deadline)?;
        if hash != reference.seal_sha256
            || seal.schema != 1
            || seal.ticket.checkpoint_id != reference.checkpoint_id
            || seal.ticket.transfer_source.root != root_path
            || now >= seal.ticket.transfer_source.expires_at
            || seal.ticket.content_cutoff > now
        {
            return Err(StagingError::Manifest);
        }
        let deadline = expiry_deadline(
            deadline,
            started,
            now,
            seal.ticket.transfer_source.expires_at,
        )?;
        let (manifest, hash, manifest_bytes): (CheckpointManifest, _, _) =
            read(directory, "manifest.json", deadline)?;
        if hash != seal.manifest_sha256 {
            return Err(StagingError::Manifest);
        }
        let reconstructed = CheckpointManifest::build(
            &seal.ticket,
            ManifestInputs {
                application_packages: manifest.application_packages.clone(),
                cli_packages: manifest.cli_packages.clone(),
                deployment: manifest.deployment.clone(),
                secret_references: manifest.secret_references.clone(),
                audit_coverage: manifest.audit_coverage,
            },
        )
        .map_err(|_| StagingError::Manifest)?;
        if reconstructed != manifest {
            return Err(StagingError::Manifest);
        }
        let (inclusion, hash, inclusion_bytes): (InclusionDocument, _, _) =
            read(directory, "inclusion.json", deadline)?;
        if hash != seal.inclusion_sha256
            || inclusion.schema != 1
            || inclusion.checkpoint_id != reference.checkpoint_id
            || tree::verify(&directory.child("data")?, deadline)? != inclusion.view
        {
            return Err(StagingError::Changed);
        }
        directory.check_path(root_path)?;
        check_deadline(deadline)?;
        let inventory = SealedInventory::from_data(
            inclusion.view,
            [
                ("seal.json", reference.seal_sha256.clone(), seal_bytes),
                ("manifest.json", seal.manifest_sha256, manifest_bytes),
                ("inclusion.json", seal.inclusion_sha256, inclusion_bytes),
            ],
            deadline,
        )?;
        Ok(Self {
            reference: reference.clone(),
            ticket: seal.ticket,
            manifest,
            inventory,
        })
    }
}
