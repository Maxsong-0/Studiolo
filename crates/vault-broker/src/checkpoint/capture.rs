//! 冻结窗口里只定格写者的两棵树（BUG-045）：逐层列目录、逐个打开文件留着句柄、记下身份，不读内容。
//!
//! 写者只用 `create_new` 新建再改名换上，从不原地改文件，所以句柄指向的 inode 内容就是冻结那一刻；
//! 放开之后写者照常删、换，旧 inode 靠这里的句柄留着。放开之后从句柄拷到独立副本、边拷边算同一套树摘要
//! （`tree.rs` 的 `DirHasher`），每个文件拷之前、拷完都 fstat 核对长度、修改时间、权限没变。
//! 不建硬链接：写者到处要求文件链接数为 1，硬链接会让放开后的正常读写报错，中途被杀还会让写者起不来。
use super::{check_deadline, copy, files, tree};
use crate::{
    BrokerError, FsVaultBroker,
    writer::{Identity, directory},
};
use cap_std::fs::{Dir, MetadataExt as _};
use std::{
    fs::{File, Metadata},
    io::Write as _,
    os::unix::fs::{FileExt as _, MetadataExt as _},
    path::Path,
    time::Instant,
};

/// 一棵定格的树：目录结构、每个文件的句柄和冻结时的身份。不引用写者实例。
pub(super) struct Captured {
    root: Vec<(String, Node)>,
    nodes: u64,
    bytes: u64,
}
enum Node {
    Directory(Vec<(String, Node)>),
    File(Held),
}
/// 冻结时打开的文件：放开之后原路径可能已经被删、被换，句柄还指着冻结那一刻的 inode。
struct Held {
    file: File,
    identity: Unchanged,
}
/// 冻结之后不该变的那几项。链接数、ctime 不在里面：放开之后写者删掉或换掉原文件会改它们，内容不变。
#[derive(PartialEq, Eq)]
struct Unchanged {
    dev: u64,
    ino: u64,
    len: u64,
    mode: u32,
    uid: u32,
    gid: u32,
    mtime: (i64, i64),
}
impl Unchanged {
    fn of(metadata: &Metadata) -> Self {
        Self {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mode: metadata.mode(),
            uid: metadata.uid(),
            gid: metadata.gid(),
            mtime: (metadata.mtime(), metadata.mtime_nsec()),
        }
    }
}

/// 两棵树一共能同时开着多少个文件：本进程打开文件数软上限的一半，另一半留给其余的连接和库。
/// 人侧启动时把软上限抬到硬上限（`local-server` 的 `startup/limits.rs`）；不够就定格失败，不半拍。
pub(super) struct Budget {
    files: u64,
    limit: u64,
}
impl Budget {
    pub(super) fn from_process() -> Self {
        let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile)
            .current
            .map_or(u64::MAX, |current| current / 2);
        Self { files: 0, limit }
    }
    /// 用例用：直接给定能同时开着多少个文件。
    #[cfg(test)]
    pub(super) fn with_limit(limit: u64) -> Self {
        Self { files: 0, limit }
    }
    fn take(&mut self) -> Result<(), BrokerError> {
        self.files += 1;
        if self.files > self.limit {
            return Err(BrokerError::WriterLimitExceeded);
        }
        Ok(())
    }
}

/// 冻结窗口里定格一棵树；`skip` 同 `tree::scan`（开库清扫的隔离区）。
pub(super) fn capture(
    root: &Dir,
    skip: &[&str],
    budget: &mut Budget,
    deadline: Instant,
) -> Result<Captured, BrokerError> {
    let mut walk = Walk {
        device: Identity::directory(root)?.dev,
        deadline,
        nodes: 0,
        bytes: 0,
        budget,
    };
    let root = walk.dir(root, skip, 0)?;
    Ok(Captured {
        root,
        nodes: walk.nodes,
        bytes: walk.bytes,
    })
}
/// 两棵树合起来不超过同一份节点数和字节数上限（和 `tree::check_pair` 同一个判断）。
pub(super) fn check_pair(left: &Captured, right: &Captured) -> Result<(), BrokerError> {
    tree::check_pair(
        &tree::TreeSeal::new([0; 32], left.nodes, left.bytes),
        &tree::TreeSeal::new([0; 32], right.nodes, right.bytes),
    )
}
struct Walk<'a> {
    device: u64,
    deadline: Instant,
    nodes: u64,
    bytes: u64,
    budget: &'a mut Budget,
}
impl Walk<'_> {
    fn dir(
        &mut self,
        source: &Dir,
        skip: &[&str],
        depth: usize,
    ) -> Result<Vec<(String, Node)>, BrokerError> {
        check_deadline(self.deadline)?;
        if depth > tree::MAX_DEPTH {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let before = source
            .dir_metadata()
            .map_err(|e| BrokerError::io("核对定格源目录", e))?;
        if !before.is_dir()
            || before.dev() != self.device
            || before.uid() != rustix::process::geteuid().as_raw()
        {
            return Err(BrokerError::IncompleteCapture);
        }
        let names = tree::listed(source, skip, tree::MAX_NODES - self.nodes, self.deadline)?;
        self.nodes += names.len() as u64;
        let mut entries = Vec::with_capacity(names.len());
        for name in &names {
            check_deadline(self.deadline)?;
            let meta = source
                .symlink_metadata(name)
                .map_err(|e| BrokerError::io("核对定格节点", e))?;
            if meta.is_dir() && !meta.file_type().is_symlink() {
                let child = directory(source, name)?;
                if Identity::directory(&child)? != Identity::file(&meta) {
                    return Err(BrokerError::CaptureChanged);
                }
                let nested = self.dir(&child, tree::within(skip, name), depth + 1)?;
                if Identity::directory(&directory(source, name)?)? != Identity::file(&meta) {
                    return Err(BrokerError::CaptureChanged);
                }
                entries.push((name.clone(), Node::Directory(nested)));
            } else if meta.is_file() && !meta.file_type().is_symlink() {
                self.budget.take()?;
                let file = files::open(source, name)?;
                let opened = files::metadata(&file, self.device)?;
                if opened.ino() != meta.ino() {
                    return Err(BrokerError::CaptureChanged);
                }
                self.bytes = self
                    .bytes
                    .checked_add(opened.len())
                    .filter(|total| *total <= tree::MAX_BYTES)
                    .ok_or(BrokerError::WriterLimitExceeded)?;
                entries.push((
                    name.clone(),
                    Node::File(Held {
                        identity: Unchanged::of(&opened),
                        file,
                    }),
                ));
            } else {
                return Err(BrokerError::IncompleteCapture);
            }
        }
        let after = source
            .dir_metadata()
            .map_err(|e| BrokerError::io("复核定格源目录", e))?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.mode() != after.mode()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || names != tree::listed(source, skip, tree::MAX_NODES, self.deadline)?
        {
            return Err(BrokerError::CaptureChanged);
        }
        check_deadline(self.deadline)?;
        Ok(entries)
    }
}

impl Captured {
    /// 窗口里复核：活的树和定格时一样（每层的名字、每个文件的 inode、长度、修改时间、权限），不读内容。
    /// 回执核对读的是活的文件；核对前后都对得上定格，核过的才是定格的那一刻。
    pub(super) fn matches(
        &self,
        root: &Dir,
        skip: &[&str],
        deadline: Instant,
    ) -> Result<(), BrokerError> {
        matches_dir(&self.root, root, skip, deadline)
    }
    /// 放开之后、动目标之前：逐个句柄 fstat，确认没有文件被原地改过（不读内容）。
    /// 拷贝途中再改的由 [`Captured::copy`] 逐个文件拷前拷后再核。
    pub(super) fn unchanged(&self, deadline: Instant) -> Result<(), BrokerError> {
        unchanged_dir(&self.root, deadline)
    }
    /// 放开之后：把定格的树拷进空的私有目标，返回按 `tree.rs` 同一套写法算的封存值。
    /// 任何一个文件拷之前或拷完和冻结时不一样（长度、修改时间、权限、inode），整份失败。
    pub(super) fn copy(
        &self,
        target: &Dir,
        deadline: Instant,
    ) -> Result<tree::TreeSeal, BrokerError> {
        let mut buffer = vec![0; 256 * 1024];
        let hash = copy_dir(&self.root, target, deadline, &mut buffer)?;
        Ok(tree::TreeSeal::new(hash, self.nodes, self.bytes))
    }
}
fn matches_dir(
    entries: &[(String, Node)],
    source: &Dir,
    skip: &[&str],
    deadline: Instant,
) -> Result<(), BrokerError> {
    check_deadline(deadline)?;
    let names = tree::listed(source, skip, tree::MAX_NODES, deadline)?;
    if names.len() != entries.len()
        || names
            .iter()
            .zip(entries)
            .any(|(name, (captured, _))| name != captured)
    {
        return Err(BrokerError::CaptureChanged);
    }
    for (name, node) in entries {
        check_deadline(deadline)?;
        match node {
            Node::Directory(children) => {
                matches_dir(
                    children,
                    &directory(source, name)?,
                    tree::within(skip, name),
                    deadline,
                )?;
            }
            Node::File(held) => {
                let file = files::open(source, name)?;
                let now = file
                    .metadata()
                    .map_err(|e| BrokerError::io("复核定格文件", e))?;
                if !now.is_file() || Unchanged::of(&now) != held.identity {
                    return Err(BrokerError::CaptureChanged);
                }
            }
        }
    }
    Ok(())
}
fn unchanged_dir(entries: &[(String, Node)], deadline: Instant) -> Result<(), BrokerError> {
    check_deadline(deadline)?;
    for (_, node) in entries {
        match node {
            Node::Directory(children) => unchanged_dir(children, deadline)?,
            Node::File(held) => unchanged(held)?,
        }
    }
    Ok(())
}
fn copy_dir(
    entries: &[(String, Node)],
    target: &Dir,
    deadline: Instant,
    buffer: &mut [u8],
) -> Result<[u8; 32], BrokerError> {
    check_deadline(deadline)?;
    let mut hash = tree::DirHasher::new(entries.len());
    for (name, node) in entries {
        match node {
            Node::Directory(children) => {
                let child = copy::dir(target, name)?;
                hash.directory(name, copy_dir(children, &child, deadline, buffer)?);
            }
            Node::File(held) => {
                let (digest, length) = copy_file(held, target, name, deadline, buffer)?;
                hash.file(name, length, digest);
            }
        }
    }
    FsVaultBroker::sync_directory(target, Path::new("."))?;
    check_deadline(deadline)?;
    Ok(hash.finish())
}
fn copy_file(
    held: &Held,
    target: &Dir,
    name: &str,
    deadline: Instant,
    buffer: &mut [u8],
) -> Result<([u8; 32], u64), BrokerError> {
    use sha2::{Digest, Sha256};
    unchanged(held)?;
    let mut output = copy::file(target, name, held.identity.len)?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    loop {
        check_deadline(deadline)?;
        let read = held
            .file
            .read_at(buffer, total)
            .map_err(|e| BrokerError::io("从定格句柄读取", e))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .filter(|total| *total <= held.identity.len)
            .ok_or(BrokerError::CaptureChanged)?;
        hash.update(&buffer[..read]);
        output
            .write_all(&buffer[..read])
            .map_err(|e| BrokerError::io("流式写入封存副本", e))?;
    }
    if total != held.identity.len {
        return Err(BrokerError::CaptureChanged);
    }
    unchanged(held)?;
    output
        .sync_all()
        .map_err(|e| BrokerError::io("同步封存文件", e))?;
    Ok((hash.finalize().into(), total))
}
/// 句柄指着的 inode 和冻结时一样：没被原地改过。
fn unchanged(held: &Held) -> Result<(), BrokerError> {
    let now = held
        .file
        .metadata()
        .map_err(|e| BrokerError::io("核对定格文件", e))?;
    if !now.is_file() || Unchanged::of(&now) != held.identity {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(())
}
