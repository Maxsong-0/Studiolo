//! 冻结窗口里只定格写者的两棵树（BUG-045）：逐层列目录、逐个打开文件留着句柄、记下身份，不读内容。
//!
//! 写者只用 `create_new` 新建再改名换上，从不原地改文件，所以句柄指向的 inode 内容就是冻结那一刻；
//! 放开之后写者照常删、换，旧 inode 靠这里的句柄留着。放开之后从句柄拷到独立副本、边拷边算同一套树摘要
//! （`tree.rs` 的 `DirHasher`），每个文件拷之前、拷完都 fstat 核对长度、修改时间、权限没变。
//! 不建硬链接：写者到处要求文件链接数为 1，硬链接会让放开后的正常读写报错，中途被杀还会让写者起不来。
use super::{check_deadline, copy, exported::Originals, files, tree};
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
use studiolo_core::Flush;

/// 一棵定格的树：目录结构、每个目录和文件冻结时的身份、每个文件的句柄。不引用写者实例。
pub(super) struct Captured {
    root: Level,
    nodes: u64,
    bytes: u64,
}
/// 一层目录：冻结那一刻的身份（写者的绑定文件里记的就是它），和按名字排好的内容。
struct Level {
    identity: Identity,
    entries: Vec<(String, Node)>,
}
enum Node {
    Directory(Level),
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
/// 人侧启动时把软上限抬到硬上限（[`super::open_files`]）；不够就定格失败，不半拍。
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
    fn dir(&mut self, source: &Dir, skip: &[&str], depth: usize) -> Result<Level, BrokerError> {
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
        Ok(Level {
            identity: Identity::file(&before),
            entries,
        })
    }
}

impl Captured {
    /// 窗口里复核：活的树和定格时一样（每层的名字和目录身份、每个文件的 inode、长度、修改时间、权限），不读内容。
    /// 定格是逐层走的：走完再对一遍，前后两棵树、先后几层才是同一刻。
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
    /// 副本里每个目录和文件对应定格时的哪一个记进 `originals`：之后在副本上核绑定文件要用。
    pub(super) fn copy(
        &self,
        target: &Dir,
        originals: &mut Originals,
        deadline: Instant,
    ) -> Result<tree::TreeSeal, BrokerError> {
        let mut copying = Copying {
            originals,
            deadline,
            buffer: vec![0; 256 * 1024],
        };
        let hash = copying.dir(&self.root, target)?;
        Ok(tree::TreeSeal::new(hash, self.nodes, self.bytes))
    }
}
fn matches_dir(
    level: &Level,
    source: &Dir,
    skip: &[&str],
    deadline: Instant,
) -> Result<(), BrokerError> {
    check_deadline(deadline)?;
    let names = tree::listed(source, skip, tree::MAX_NODES, deadline)?;
    if Identity::directory(source)? != level.identity
        || names.len() != level.entries.len()
        || names
            .iter()
            .zip(&level.entries)
            .any(|(name, (captured, _))| name != captured)
    {
        return Err(BrokerError::CaptureChanged);
    }
    for (name, node) in &level.entries {
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
fn unchanged_dir(level: &Level, deadline: Instant) -> Result<(), BrokerError> {
    check_deadline(deadline)?;
    for (_, node) in &level.entries {
        match node {
            Node::Directory(children) => unchanged_dir(children, deadline)?,
            Node::File(held) => unchanged(held)?,
        }
    }
    Ok(())
}
/// 拷一棵树要带着走的：副本对原件、截止时间、流式缓冲。
struct Copying<'a> {
    originals: &'a mut Originals,
    deadline: Instant,
    buffer: Vec<u8>,
}
impl Copying<'_> {
    fn dir(&mut self, level: &Level, target: &Dir) -> Result<[u8; 32], BrokerError> {
        check_deadline(self.deadline)?;
        self.originals
            .record(Identity::directory(target)?, level.identity)?;
        let mut hash = tree::DirHasher::new(level.entries.len());
        for (name, node) in &level.entries {
            match node {
                Node::Directory(children) => {
                    let child = copy::dir(target, name)?;
                    hash.directory(name, self.dir(children, &child)?);
                }
                Node::File(held) => {
                    let (digest, length) = self.file(held, target, name)?;
                    hash.file(name, length, digest);
                }
            }
        }
        FsVaultBroker::sync_directory(target, Path::new("."))?;
        check_deadline(self.deadline)?;
        Ok(hash.finish())
    }
    fn file(
        &mut self,
        held: &Held,
        target: &Dir,
        name: &str,
    ) -> Result<([u8; 32], u64), BrokerError> {
        use sha2::{Digest, Sha256};
        unchanged(held)?;
        let mut output = copy::file(target, name, held.identity.len)?;
        let mut hash = Sha256::new();
        let mut total = 0u64;
        // 一个文件一份计数：没落盘的量封顶，不随文件大小涨（BUG-054）。
        let mut flush = Flush::default();
        loop {
            check_deadline(self.deadline)?;
            let read = held
                .file
                .read_at(&mut self.buffer, total)
                .map_err(|e| BrokerError::io("从定格句柄读取", e))?;
            if read == 0 {
                break;
            }
            total = total
                .checked_add(read as u64)
                .filter(|total| *total <= held.identity.len)
                .ok_or(BrokerError::CaptureChanged)?;
            hash.update(&self.buffer[..read]);
            output
                .write_all(&self.buffer[..read])
                .map_err(|e| BrokerError::io("流式写入封存副本", e))?;
            flush.written(read as u64, || flush_halfway(&output))?;
        }
        if total != held.identity.len {
            return Err(BrokerError::CaptureChanged);
        }
        unchanged(held)?;
        output
            .sync_all()
            .map_err(|e| BrokerError::io("同步封存文件", e))?;
        let written = output
            .metadata()
            .map_err(|e| BrokerError::io("核对封存副本文件", e))?;
        self.originals.record(
            Identity {
                dev: written.dev(),
                ino: written.ino(),
            },
            Identity {
                dev: held.identity.dev,
                ino: held.identity.ino,
            },
        )?;
        Ok((hash.finalize().into(), total))
    }
}
/// 写到一半先落一次盘：没落盘的页在有内存上限的服务里回收不了，攒到写完才落，量就跟着文件大小走（BUG-054）。
fn flush_halfway(output: &File) -> Result<(), BrokerError> {
    output
        .sync_data()
        .map_err(|e| BrokerError::io("封存副本写到一半落盘", e))?;
    #[cfg(test)]
    {
        let length = output
            .metadata()
            .map_err(|e| BrokerError::io("核对封存副本文件", e))?
            .len();
        FLUSHED.with_borrow_mut(|lengths| lengths.push(length));
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// 用例用：本线程写到一半每次落盘时，文件有多长。
    pub(super) static FLUSHED: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
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
