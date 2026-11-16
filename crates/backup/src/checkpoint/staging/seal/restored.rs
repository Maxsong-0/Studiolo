//! 从远端取回的封存副本：放在任意私有目录里、过了保留期也能只读核验，但不能当上传来源。
//!
//! 和 [`super::SealedCheckpoint`] 是两个类型：追加执行器只收后者，取回的副本换不成上传能力。
//! 这里也是把封存内容放到新目录的唯一出口：写出去的每个字节都对着已核验视图里的摘要。
use super::super::{
    Directory, MAX_NODES, Result, StagingError, check_deadline, hex,
    io::Regular,
    tree::{self, FileProof, TreeProof},
};
use super::{DiskSeal, InclusionDocument, SealedReference, read, read::consistent};
use crate::checkpoint::manifest::CheckpointManifest;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use studiolo_core::VaultPath;

/// 一次放置的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// 这次逐字节核对着复制出来的。
    Extracted,
    /// 目标名字已经存在：这次什么都没写，也没有核对它的内容。
    AlreadyPresent,
}

/// 经实际核验的取回副本；没有公开字段，也没有反序列化入口。
pub struct RestoredCheckpoint {
    root: PathBuf,
    reference: SealedReference,
    manifest: CheckpointManifest,
    view: TreeProof,
}

impl RestoredCheckpoint {
    /// 核验一份取回的封存副本。
    ///
    /// 引用必须来自可信记录（人侧账本的准入事件，或远端最终清单），不能取自副本自己。副本目录必须
    /// 属于当前账号、整棵只读、没有 ACL。和重开原封存相比只少两条：不要求目录还在原来的路径上，
    /// 不要求还在保留期内。
    pub fn open(root: &Path, reference: &SealedReference, deadline: Instant) -> Result<Self> {
        check_deadline(deadline)?;
        let directory = Directory::open(root, true)?;
        directory.require_readonly()?;
        if directory.names(5)? != ["data", "inclusion.json", "manifest.json", "seal.json"] {
            return Err(StagingError::Manifest);
        }
        let (seal, hash, _): (DiskSeal, _, _) = read(&directory, "seal.json", deadline)?;
        if hash != reference.seal_sha256
            || seal.schema != 1
            || seal.ticket.checkpoint_id != reference.checkpoint_id
        {
            return Err(StagingError::Manifest);
        }
        let (manifest, hash, _): (CheckpointManifest, _, _) =
            read(&directory, "manifest.json", deadline)?;
        if hash != seal.manifest_sha256 {
            return Err(StagingError::Manifest);
        }
        consistent(&seal.ticket, &manifest)?;
        let (inclusion, hash, _): (InclusionDocument, _, _) =
            read(&directory, "inclusion.json", deadline)?;
        if hash != seal.inclusion_sha256
            || inclusion.schema != 1
            || inclusion.checkpoint_id != reference.checkpoint_id
            || tree::verify(&directory.child("data")?, deadline)? != inclusion.view
        {
            return Err(StagingError::Changed);
        }
        directory.check_path(root)?;
        check_deadline(deadline)?;
        Ok(Self {
            root: root.to_owned(),
            reference: reference.clone(),
            manifest,
            view: inclusion.view,
        })
    }
    /// 核验时用的原封存引用。
    #[must_use]
    pub fn reference(&self) -> &SealedReference {
        &self.reference
    }
    /// 已核验的清单：owner 名单和各自的水位都从这里读。
    #[must_use]
    pub fn manifest(&self) -> &CheckpointManifest {
        &self.manifest
    }
    /// 目标名字是不是已经在父目录里（不跟随链接）。
    pub fn placed(target_parent: &Path, name: &str) -> Result<bool> {
        Directory::open(target_parent, true)?.exists(name)
    }
    /// 上一次放置 `name` 中途失败留下的 `<name>.restoring` 还在不在（不跟随链接）。它在场时
    /// [`Self::extract`] 一定拒绝，所以编排要在写任何东西之前先问这一句。
    pub fn interrupted(target_parent: &Path, name: &str) -> Result<bool> {
        Directory::open(target_parent, true)?.exists(&partial_name(name))
    }
    /// 新根恢复（T-212）：把先放在暂用名 `held` 下的组件改成正式名 `name`。同一父目录里原子改名，正式名
    /// 已经在就失败、不覆盖，改完同步父目录，再核父目录还是同一个。
    ///
    /// 带凭据的库（Auth、身份库）先放在暂用名下：它的 owner 把旧凭据全部作废之前，按正式路径谁都打不开
    /// （包括不经启动闸门的只读模式和 agent-server）。这里不核内容：暂用名下的库这时已经被它的 owner 改过。
    pub fn publish_held(target_parent: &Path, held: &str, name: &str) -> Result<()> {
        let parent = Directory::open(target_parent, true)?;
        parent.publish(held, name)?;
        parent.check_path(target_parent)
    }
    /// 把 `data/<component>` 逐字节核对着复制成 `target_parent/<name>`。
    ///
    /// 写的每个文件都对着已核验视图里的摘要和长度，副本里多一个、少一个节点都失败。先写进
    /// `<name>.restoring`，全部落盘后才原子改名；中途失败留下的 `.restoring` 不自动清，下次直接拒绝
    /// （[`StagingError::CleanupRequired`]）。目标名字已经存在时什么都不写。
    /// 新目录是属主可写的（目录 0700、文件 0600），因为各库接下来要在上面继续写。
    pub fn extract(
        &self,
        component: &str,
        target_parent: &Path,
        name: &str,
        deadline: Instant,
    ) -> Result<Placement> {
        check_deadline(deadline)?;
        if !self.view.directories.contains(component) {
            return Err(StagingError::Catalog);
        }
        let target = Directory::open(target_parent, true)?;
        if target.exists(name)? {
            return Ok(Placement::AlreadyPresent);
        }
        let partial = partial_name(name);
        if target.exists(&partial)? {
            return Err(StagingError::CleanupRequired);
        }
        let root = Directory::open(&self.root, true)?;
        let source = root.child("data")?.child(component)?;
        let output = target.create_dir(&partial)?;
        let mut replica = Replica {
            expected: &self.view,
            deadline,
            buffer: vec![0; 256 * 1024],
            directories: 0,
            files: 0,
        };
        replica.directory(&source, &output, component, 0)?;
        // 视图里这个组件下面的每个节点都必须真的复制过；只核对「复制的都对」还不够。
        let prefix = format!("{component}/");
        let directories = self
            .view
            .directories
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .count();
        let files = self
            .view
            .files
            .keys()
            .filter(|path| path.as_str().starts_with(&prefix))
            .count();
        if replica.directories != directories || replica.files != files {
            return Err(StagingError::Changed);
        }
        output.sync()?;
        root.check_path(&self.root)?;
        check_deadline(deadline)?;
        target.publish(&partial, name)?;
        target.check_path(target_parent)?;
        Ok(Placement::Extracted)
    }
    /// 接手上一次时重核已经放好的 `target_parent/<name>`（D2-F2）：和 [`Self::extract`] 对着同一份
    /// 已核验视图，多一个、少一个节点或改了一个字节都失败（[`StagingError::Changed`]）。`nested` 是放在它
    /// 第一层里的别的组件的名字（接入方账本放在身份库目录里的 `audit`），只跳过这些名字本身。只读。
    pub fn verify_placed(
        &self,
        component: &str,
        target_parent: &Path,
        name: &str,
        nested: &[&str],
        deadline: Instant,
    ) -> Result<()> {
        check_deadline(deadline)?;
        if !self.view.directories.contains(component) {
            return Err(StagingError::Catalog);
        }
        let parent = Directory::open(target_parent, true)?;
        let placed = parent.child(name)?;
        let mut check = Placed {
            expected: &self.view,
            deadline,
            buffer: vec![0; 256 * 1024],
            directories: 0,
            files: 0,
        };
        check.directory(&placed, component, nested, 0)?;
        let prefix = format!("{component}/");
        let directories = self
            .view
            .directories
            .iter()
            .filter(|path| path.starts_with(&prefix))
            .count();
        let files = self
            .view
            .files
            .keys()
            .filter(|path| path.as_str().starts_with(&prefix))
            .count();
        if check.directories != directories || check.files != files {
            return Err(StagingError::Changed);
        }
        placed.check_child(&parent, name)?;
        parent.check_path(target_parent)?;
        check_deadline(deadline)
    }
}

/// 放置 `name` 时先写进的半成品目录名；全部落盘后才改名成 `name`。
fn partial_name(name: &str) -> String {
    format!("{name}.restoring")
}

/// 一次放置用的遍历状态；只认已核验视图里登记过的节点。
struct Replica<'a> {
    expected: &'a TreeProof,
    deadline: Instant,
    buffer: Vec<u8>,
    directories: usize,
    files: usize,
}
impl Replica<'_> {
    fn directory(
        &mut self,
        source: &Directory,
        target: &Directory,
        prefix: &str,
        depth: usize,
    ) -> Result<()> {
        check_deadline(self.deadline)?;
        if depth > 64 {
            return Err(StagingError::Limit);
        }
        source.require_readonly()?;
        let names = source.names(MAX_NODES)?;
        let expected = self.expected;
        for name in &names {
            check_deadline(self.deadline)?;
            let path = format!("{prefix}/{name}");
            if source.is_directory(name)? {
                if !expected.directories.contains(&path) {
                    return Err(StagingError::Changed);
                }
                self.directories += 1;
                let child = source.child(name)?;
                let output = target.create_dir(name)?;
                self.directory(&child, &output, &path, depth + 1)?;
                child.check_child(source, name)?;
                output.sync()?;
            } else {
                let path = VaultPath::parse(path).map_err(|_| StagingError::Catalog)?;
                let proof = expected.files.get(&path).ok_or(StagingError::Changed)?;
                self.file(source, target, name, proof)?;
                self.files += 1;
            }
        }
        if source.names(MAX_NODES)? != names {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    fn file(
        &mut self,
        source: &Directory,
        target: &Directory,
        name: &str,
        expected: &FileProof,
    ) -> Result<()> {
        let mut input = Regular::open(source, name)?;
        input.require_readonly()?;
        if input.len() != expected.bytes {
            return Err(StagingError::Changed);
        }
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
        if size != expected.bytes || hex(&hash.finalize()) != expected.sha256 {
            return Err(StagingError::Changed);
        }
        output.finish_writable()
    }
}

/// 重核已放置组件用的只读遍历状态；只认已核验视图里登记过的节点，不写任何东西。
struct Placed<'a> {
    expected: &'a TreeProof,
    deadline: Instant,
    buffer: Vec<u8>,
    directories: usize,
    files: usize,
}
impl Placed<'_> {
    fn directory(
        &mut self,
        current: &Directory,
        prefix: &str,
        skip: &[&str],
        depth: usize,
    ) -> Result<()> {
        check_deadline(self.deadline)?;
        if depth > 64 {
            return Err(StagingError::Limit);
        }
        let names = current.names(MAX_NODES)?;
        for name in names.iter().filter(|name| !skip.contains(&name.as_str())) {
            check_deadline(self.deadline)?;
            let path = format!("{prefix}/{name}");
            if current.is_directory(name)? {
                if !self.expected.directories.contains(&path) {
                    return Err(StagingError::Changed);
                }
                self.directories += 1;
                let child = current.child(name)?;
                self.directory(&child, &path, &[], depth + 1)?;
                child.check_child(current, name)?;
            } else {
                let path = VaultPath::parse(path).map_err(|_| StagingError::Catalog)?;
                let proof = self
                    .expected
                    .files
                    .get(&path)
                    .ok_or(StagingError::Changed)?;
                self.file(current, name, proof)?;
                self.files += 1;
            }
        }
        if current.names(MAX_NODES)? != names {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
    fn file(&mut self, parent: &Directory, name: &str, expected: &FileProof) -> Result<()> {
        let mut input = Regular::open(parent, name)?;
        if input.len() != expected.bytes {
            return Err(StagingError::Changed);
        }
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
        }
        input.check(parent, name)?;
        if size != expected.bytes || hex(&hash.finalize()) != expected.sha256 {
            return Err(StagingError::Changed);
        }
        Ok(())
    }
}
