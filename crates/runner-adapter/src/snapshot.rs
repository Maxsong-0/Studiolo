//! 宿主机持有的 git 快照，以及候选补丁的持久保留；绝不更新任何分支。
use crate::{LaunchStage, RunnerError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PATCH: usize = 16 * 1024 * 1024;
/// 随二进制补丁一同携带的不可变候选元数据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchMetadata {
    /// 基线快照 tree，含已暂存、未暂存与未跟踪的内容。
    pub base_tree: String,
    /// 隔离进程停止后观测到的结果 tree。
    pub after_tree: String,
    /// 二进制补丁的 SHA-256。
    pub sha256: String,
    /// 宿主机持有的已保留候选补丁，绝不是可丢弃工作树内的路径。
    pub retained_path: PathBuf,
}
/// 可信的宿主机快照。只有经批准的宿主机路径才能进入此 API，绝不从 Runner 请求中
/// 反序列化这些路径。源仓库的 Git 对象库保存不可变、可共享的基线 tree。
pub struct CodeSnapshot {
    repo: PathBuf,
    /// 每个 attempt 独有的可写副本；绝不与其他 attempt 共享。
    pub work: PathBuf,
    index: PathBuf,
    /// 精确、完整的输入 tree，含未提交与未跟踪的内容。
    pub base: String,
}
fn git(repo: &Path, work: &Path, index: &Path, args: &[&str]) -> Result<Vec<u8>, RunnerError> {
    let result = Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_INDEX_FILE", index)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Studiolo snapshot")
        .env("GIT_AUTHOR_EMAIL", "snapshot@invalid")
        .env("GIT_COMMITTER_NAME", "Studiolo snapshot")
        .env("GIT_COMMITTER_EMAIL", "snapshot@invalid")
        .arg("--git-dir")
        .arg(repo)
        .arg("--work-tree")
        .arg(work)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "diff.external=",
            "-c",
            "core.pager=cat",
        ])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| RunnerError::LaunchFailed {
            stage: LaunchStage::Snapshot,
        })?;
    if !result.status.success() {
        return Err(RunnerError::LaunchFailed {
            stage: LaunchStage::Snapshot,
        });
    }
    Ok(result.stdout)
}
fn oid(bytes: Vec<u8>) -> Result<String, RunnerError> {
    let value = String::from_utf8(bytes).map_err(|_| RunnerError::InvalidExecution)?;
    let value = value.trim();
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(RunnerError::InvalidExecution);
    }
    Ok(value.to_owned())
}
pub(crate) type Snapshot = CodeSnapshot;
impl CodeSnapshot {
    /// 捕获经批准的完整检出，并创建一份隔离的可写副本。
    /// 运行期间，调用方必须持有该项目的源码／落地租约。
    pub fn create(source: &Path, attempt: &Path) -> Result<Self, RunnerError> {
        let source = canonical_directory(source)?;
        let attempt = canonical_directory(attempt)?;
        let root = attempt.parent().ok_or(RunnerError::InvalidExecution)?;
        let lease = root_lock(root)?;
        if fs::read_dir(&attempt)
            .map_err(|e| RunnerError::io("new snapshot directory", e))?
            .next()
            .is_some()
        {
            return Err(RunnerError::InvalidExecution);
        }
        private_directory(&attempt, lease.owner)?;
        let result = Command::new("git")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["-C"])
            .arg(&source)
            .args(["rev-parse", "--absolute-git-dir"])
            .output()
            .map_err(|_| RunnerError::LaunchFailed {
                stage: LaunchStage::Snapshot,
            })?;
        if !result.status.success() {
            return Err(RunnerError::InvalidExecution);
        }
        let repo = PathBuf::from(
            String::from_utf8(result.stdout)
                .map_err(|_| RunnerError::InvalidExecution)?
                .trim(),
        );
        let repo = canonical_directory(&repo)?;
        let index = attempt.join("snapshot.index");
        // 本地 include 与 clean/smudge 过滤器会在 git add 时执行宿主机代码，故一律拒绝。
        let config = git(&repo, &source, &index, &["config", "--local", "--list"])?;
        if String::from_utf8_lossy(&config).lines().any(|line| {
            line.starts_with("filter.")
                || line.starts_with("include.")
                || line.starts_with("includeif.")
        }) {
            return Err(RunnerError::InvalidExecution);
        }
        git(&repo, &source, &index, &["read-tree", "HEAD"])?;
        git(&repo, &source, &index, &["add", "-A", "--", "."])?;
        let base = oid(git(&repo, &source, &index, &["write-tree"])?)?;
        let head = oid(git(&repo, &source, &index, &["rev-parse", "HEAD"])?)?;
        let commit = shared_commit(root, &repo, &source, &index, &base, &head, lease.owner)?;
        let work = attempt.join("work");
        let work_text = work.to_str().ok_or(RunnerError::InvalidExecution)?;
        git(
            &repo,
            &source,
            &index,
            &["worktree", "add", "--detach", work_text, &commit],
        )?;
        // 锁定工作树：删除沙箱内的 .git 文件后，Git 也不能 prune 掉这条仍在使用的登记。
        git(
            &repo,
            &source,
            &index,
            &[
                "worktree",
                "lock",
                "--reason",
                "Studiolo retained attempt",
                work_text,
            ],
        )?;
        let admin_text = fs::read_to_string(work.join(".git"))
            .map_err(|e| RunnerError::io("snapshot registration", e))?;
        let admin = canonical_directory(Path::new(
            admin_text
                .trim()
                .strip_prefix("gitdir: ")
                .ok_or(RunnerError::InvalidExecution)?,
        ))?;
        let common = PathBuf::from(
            String::from_utf8(git(
                &repo,
                &source,
                &index,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )?)
            .map_err(|_| RunnerError::InvalidExecution)?
            .trim(),
        );
        if admin.parent() != Some(common.join("worktrees").as_path()) {
            return Err(RunnerError::InvalidExecution);
        }
        // 沙箱能看到文件，但绝看不到任何 git 控制路径或可执行的项目配置。
        for name in [".git", ".codex", ".claude", ".mcp.json"] {
            let path = work.join(name);
            if path.is_dir() {
                fs::remove_dir_all(path)
            } else if path.exists() {
                fs::remove_file(path)
            } else {
                Ok(())
            }
            .map_err(|error| RunnerError::io("exclude runner control path", error))?;
        }
        let record = SnapshotRecord {
            schema: 1,
            source: source.clone(),
            repo: repo.clone(),
            base: base.clone(),
            created_at: unix_seconds()?,
            source_identity: identity(&source)?,
            root_identity: identity(root)?,
            attempt_identity: identity(&attempt)?,
            work_identity: identity(&work)?,
            repo_identity: identity(&repo)?,
            admin: admin.clone(),
            admin_identity: identity(&admin)?,
            retained: None,
        };
        write_record(&attempt.join(RECORD), &record, lease.owner, false)?;
        Ok(Self {
            repo,
            work,
            index,
            base,
        })
    }
    /// 持久保留二进制补丁；含冲突标记的候选同样会保留，但随后被拒绝。
    pub fn preserve(&self, retained: &Path) -> Result<(PatchMetadata, Vec<u8>), RunnerError> {
        // 枚举候选时直接拒绝非普通文件、硬链接与控制路径；冲突标记只记下，先保留补丁再拒绝。
        let conflict_free = validate_tree(&self.work)?;
        git(
            &self.repo,
            &self.work,
            &self.index,
            &["read-tree", &self.base],
        )?;
        git(
            &self.repo,
            &self.work,
            &self.index,
            &[
                "add",
                "-A",
                "--",
                ".",
                ":(exclude).git",
                ":(exclude).codex",
                ":(exclude).claude",
                ":(exclude).mcp.json",
            ],
        )?;
        let after = oid(git(&self.repo, &self.work, &self.index, &["write-tree"])?)?;
        let patch = git(
            &self.repo,
            &self.work,
            &self.index,
            &[
                "diff",
                "--binary",
                "--no-ext-diff",
                "--no-textconv",
                &self.base,
                &after,
            ],
        )?;
        if patch.len() > MAX_PATCH {
            return Err(RunnerError::OutputLimit);
        }
        let metadata = PatchMetadata {
            base_tree: self.base.clone(),
            after_tree: after,
            sha256: Sha256::digest(&patch)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            retained_path: retained.to_path_buf(),
        };
        durable_bytes(retained, &patch)?;
        durable_metadata(&metadata)?;
        if !conflict_free {
            return Err(RunnerError::CandidateRejected {
                retained: retained.to_path_buf(),
            });
        }
        Ok((metadata, patch))
    }
}
fn validate_tree(root: &Path) -> Result<bool, RunnerError> {
    let mut conflict_free = true;
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0u64;
    let mut nodes = 0usize;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir).map_err(|e| RunnerError::io("candidate scan", e))? {
            let entry = entry.map_err(|e| RunnerError::io("candidate entry", e))?;
            nodes += 1;
            if nodes > 100_000 {
                return Err(RunnerError::OutputLimit);
            }
            let name = entry.file_name();
            if [".git", ".codex", ".claude", ".mcp.json"]
                .iter()
                .any(|v| name == *v)
            {
                return Err(RunnerError::InvalidExecution);
            }
            let meta = entry
                .path()
                .symlink_metadata()
                .map_err(|e| RunnerError::io("candidate metadata", e))?;
            if meta.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !meta.is_file() {
                return Err(RunnerError::InvalidExecution);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if meta.nlink() != 1 {
                    return Err(RunnerError::InvalidExecution);
                }
            }
            total = total.saturating_add(meta.len());
            if total > 512 * 1024 * 1024 {
                return Err(RunnerError::OutputLimit);
            }
            let bytes = fs::read(entry.path()).map_err(|e| RunnerError::io("candidate read", e))?;
            if bytes
                .split(|b| *b == b'\n')
                .any(|line| line.starts_with(b"<<<<<<< ") || line.starts_with(b">>>>>>> "))
            {
                conflict_free = false;
            }
        }
    }
    Ok(conflict_free)
}

const RECORD: &str = ".studiolo-snapshot.json";
const CACHE: &str = ".studiolo-snapshot-cache.json";
const LOCK: &str = ".studiolo-snapshot.lock";
const CACHE_SECONDS: u64 = 60;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
    owner: u32,
}
#[cfg(unix)]
fn identity(path: &Path) -> Result<Identity, RunnerError> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(path).map_err(|e| RunnerError::io("snapshot identity", e))?;
    if !m.is_dir() {
        return Err(RunnerError::InvalidExecution);
    }
    Ok(Identity {
        device: m.dev(),
        inode: m.ino(),
        owner: m.uid(),
    })
}
#[cfg(not(unix))]
fn identity(_: &Path) -> Result<Identity, RunnerError> {
    Err(RunnerError::Unavailable)
}
fn canonical_directory(path: &Path) -> Result<PathBuf, RunnerError> {
    if !path.is_absolute() || path.canonicalize().ok().as_deref() != Some(path) {
        return Err(RunnerError::InvalidExecution);
    }
    identity(path)?;
    Ok(path.to_path_buf())
}
#[cfg(unix)]
fn private_directory(path: &Path, owner: u32) -> Result<(), RunnerError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let meta =
        fs::symlink_metadata(path).map_err(|e| RunnerError::io("private snapshot directory", e))?;
    if !meta.is_dir() || meta.uid() != owner {
        return Err(RunnerError::InvalidExecution);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| RunnerError::io("restrict snapshot directory", e))
}
#[cfg(not(unix))]
fn private_directory(_: &Path, _: u32) -> Result<(), RunnerError> {
    Err(RunnerError::Unavailable)
}
struct RootLock {
    _file: File,
    owner: u32,
    stamp: Identity,
}
impl Drop for RootLock {
    fn drop(&mut self) {
        // 并发的 fork 可能在 exec 之前暂时持有这个已打开的文件描述（open file description）。
        // 显式 unlock 立即结束我们的租约，而不必等每个复制出的 FD 都关闭。
        let _ = self._file.unlock();
    }
}
fn root_lock(root: &Path) -> Result<RootLock, RunnerError> {
    canonical_directory(root)?;
    let stamp = identity(root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let meta = fs::metadata(root).map_err(|e| RunnerError::io("snapshot root", e))?;
        if meta.mode() & 0o077 != 0 {
            return Err(RunnerError::InvalidExecution);
        }
        // 新建一个私有文件即可证明当前的有效 UID，无需调用进程 API。
        let probe = tempfile::NamedTempFile::new_in(root)
            .map_err(|e| RunnerError::io("snapshot owner", e))?;
        if probe
            .as_file()
            .metadata()
            .map_err(|e| RunnerError::io("snapshot owner", e))?
            .uid()
            != stamp.owner
        {
            return Err(RunnerError::InvalidExecution);
        }
        let path = root.join(LOCK);
        if entry_exists(&path)? {
            regular(&path, stamp.owner)?;
        }
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|e| RunnerError::io("snapshot lock", e))?;
        let meta = file
            .metadata()
            .map_err(|e| RunnerError::io("snapshot lock", e))?;
        let observed = regular(&path, stamp.owner)?;
        if meta.dev() != observed.dev() || meta.ino() != observed.ino() {
            return Err(RunnerError::InvalidExecution);
        }
        file.try_lock().map_err(|_| RunnerError::StateUnavailable)?;
        if identity(root)? != stamp {
            return Err(RunnerError::InvalidExecution);
        }
        Ok(RootLock {
            _file: file,
            owner: stamp.owner,
            stamp,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = stamp;
        Err(RunnerError::Unavailable)
    }
}
#[cfg(unix)]
fn regular(path: &Path, owner: u32) -> Result<fs::Metadata, RunnerError> {
    use std::os::unix::fs::MetadataExt;
    let m =
        fs::symlink_metadata(path).map_err(|e| RunnerError::io("snapshot record identity", e))?;
    if !m.is_file() || m.nlink() != 1 || m.uid() != owner || m.mode() & 0o077 != 0 {
        return Err(RunnerError::InvalidExecution);
    }
    Ok(m)
}
#[cfg(not(unix))]
fn regular(_: &Path, _: u32) -> Result<fs::Metadata, RunnerError> {
    Err(RunnerError::Unavailable)
}
fn entry_exists(path: &Path) -> Result<bool, RunnerError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(RunnerError::io("snapshot entry", e)),
    }
}
fn durable_metadata(metadata: &PatchMetadata) -> Result<(), RunnerError> {
    let parent = metadata
        .retained_path
        .parent()
        .ok_or(RunnerError::InvalidExecution)?;
    let lock = root_lock(parent)?;
    let name = metadata
        .retained_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(RunnerError::InvalidExecution)?;
    let path = parent.join(format!("{name}.json"));
    if let Some(existing) = read_record::<PatchMetadata>(&path, lock.owner)? {
        if existing != *metadata {
            return Err(RunnerError::IdempotencyConflict);
        }
        return sync_directory(parent);
    }
    write_record(&path, metadata, lock.owner, false)
}
fn unix_seconds() -> Result<u64, RunnerError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| RunnerError::InvalidExecution)
}
fn read_record<T: serde::de::DeserializeOwned>(
    path: &Path,
    owner: u32,
) -> Result<Option<T>, RunnerError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RunnerError::io("snapshot record", e)),
        Ok(_) => {
            regular(path, owner)?;
        }
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| RunnerError::io("snapshot record", e))?
        .take(32_769)
        .read_to_end(&mut bytes)
        .map_err(|e| RunnerError::io("snapshot record", e))?;
    if bytes.len() > 32_768 {
        return Err(RunnerError::OutputLimit);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| RunnerError::InvalidExecution)
}
fn sync_directory(path: &Path) -> Result<(), RunnerError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| RunnerError::io("sync snapshot directory", e))
}
fn write_record<T: Serialize>(
    path: &Path,
    record: &T,
    owner: u32,
    replace: bool,
) -> Result<(), RunnerError> {
    let parent = path.parent().ok_or(RunnerError::InvalidExecution)?;
    let stamp = identity(parent)?;
    if stamp.owner != owner {
        return Err(RunnerError::InvalidExecution);
    }
    if entry_exists(path)? {
        regular(path, owner)?;
    }
    let bytes = serde_json::to_vec(record).map_err(|_| RunnerError::InvalidExecution)?;
    if bytes.len() > 32_768 {
        return Err(RunnerError::OutputLimit);
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| RunnerError::io("snapshot record stage", e))?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|e| RunnerError::io("sync snapshot record", e))?;
    if identity(parent)? != stamp {
        return Err(RunnerError::InvalidExecution);
    }
    if replace {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    }
    .map_err(|e| RunnerError::io("install snapshot record", e.error))?;
    sync_directory(parent)
}
fn durable_bytes(path: &Path, bytes: &[u8]) -> Result<(), RunnerError> {
    let parent = path.parent().ok_or(RunnerError::InvalidExecution)?;
    let lock = root_lock(parent)?;
    if entry_exists(path)? {
        regular(path, lock.owner)?;
        let mut existing = Vec::new();
        File::open(path)
            .map_err(|e| RunnerError::io("retained patch", e))?
            .take(MAX_PATCH as u64 + 1)
            .read_to_end(&mut existing)
            .map_err(|e| RunnerError::io("retained patch", e))?;
        if existing != bytes {
            return Err(RunnerError::IdempotencyConflict);
        }
        File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|e| RunnerError::io("sync retained patch", e))?;
    } else {
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| RunnerError::io("retained patch stage", e))?;
        temporary
            .write_all(bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|e| RunnerError::io("sync retained patch", e))?;
        temporary
            .persist_noclobber(path)
            .map_err(|e| RunnerError::io("retain patch", e.error))?;
    }
    if identity(parent)? != lock.stamp {
        return Err(RunnerError::InvalidExecution);
    }
    sync_directory(parent)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CacheRecord {
    source: PathBuf,
    repo: PathBuf,
    base: String,
    head: String,
    commit: String,
    at: u64,
}
fn shared_commit(
    root: &Path,
    repo: &Path,
    source: &Path,
    index: &Path,
    base: &str,
    head: &str,
    owner: u32,
) -> Result<String, RunnerError> {
    let now = unix_seconds()?;
    if let Some(cache) = read_record::<CacheRecord>(&root.join(CACHE), owner)?
        && cache.source == source
        && cache.repo == repo
        && cache.base == base
        && cache.head == head
        && now
            .checked_sub(cache.at)
            .is_some_and(|age| age <= CACHE_SECONDS)
    {
        oid(cache.commit.as_bytes().to_vec())?;
        let actual = git(
            repo,
            source,
            index,
            &["show", "--format=%T%x00%P", "--no-patch", &cache.commit],
        )?;
        if actual == format!("{base}\0{head}\n").as_bytes() {
            return Ok(cache.commit);
        }
        return Err(RunnerError::InvalidExecution);
    }
    let commit = oid(git(
        repo,
        source,
        index,
        &[
            "commit-tree",
            base,
            "-p",
            head,
            "-m",
            "Studiolo immutable input snapshot",
        ],
    )?)?;
    write_record(
        &root.join(CACHE),
        &CacheRecord {
            source: source.to_path_buf(),
            repo: repo.to_path_buf(),
            base: base.into(),
            head: head.into(),
            commit: commit.clone(),
            at: now,
        },
        owner,
        true,
    )?;
    Ok(commit)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotRecord {
    schema: u8,
    source: PathBuf,
    repo: PathBuf,
    base: String,
    created_at: u64,
    source_identity: Identity,
    root_identity: Identity,
    attempt_identity: Identity,
    work_identity: Identity,
    repo_identity: Identity,
    admin: PathBuf,
    admin_identity: Identity,
    retained: Option<PatchMetadata>,
}
impl CodeSnapshot {
    /// 任何可信落地之后（即使是空操作的落地），都要作废短期的基线缓存元数据。
    /// 不可变的 Git 对象与已绑定的 attempt 不会被删除或改写。
    pub fn invalidate_cache(attempt_root: &Path) -> Result<(), RunnerError> {
        let lock = root_lock(attempt_root)?;
        let path = attempt_root.join(CACHE);
        if entry_exists(&path)? {
            regular(&path, lock.owner)?;
            fs::remove_file(path).map_err(|e| RunnerError::io("invalidate snapshot cache", e))?;
            sync_directory(attempt_root)?;
        }
        Ok(())
    }
}

/// 可信的维护输入。调用前须持有独占的准入／落地维护窗口，并先隔离旧的 scope；
/// 本函数不会根据目录年龄推断进程是否存活。
pub struct SnapshotCleanupRequest<'a> {
    /// 规范化的、由管理员映射的源仓库检出目录。
    pub source_checkout: &'a Path,
    /// 规范化的私有根目录，只包含各自登记过的 attempt 目录。
    pub attempt_root: &'a Path,
    /// 规范化的私有持久目录，位于源码根与 scratch 根之外。
    pub retained_root: &'a Path,
    /// 当前权威的活动 attempt 目录路径；这些目录始终受保护。
    pub active_attempts: &'a [PathBuf],
    /// 可信的宿主机时间（Unix 秒）；创建时间在未来的记录一律保留不清理。
    pub now_unix_seconds: u64,
    /// 最小年龄，至少五分钟；过新或仍在创建中的条目绝不删除。
    pub minimum_age_seconds: u64,
}
/// 可观测的维护结果；被阻断的条目留待运维显式恢复。
#[derive(Debug, Default)]
pub struct SnapshotCleanupReport {
    /// 已完整删除的已登记工作副本，及其精确对应的 Git 工作树登记。
    pub removed: Vec<PathBuf>,
    /// 已持久保留的补丁证据，包括随后清理被阻断的条目。
    pub retained: Vec<PatchMetadata>,
    /// 因活动、过新或创建时间在未来而未被触碰的目录。
    pub protected: Vec<PathBuf>,
    /// 被阻断的条目：身份未知或已变化、目录树不安全、补丁保留或删除失败；清理止步于出错处。
    pub blocked: Vec<PathBuf>,
}
/// 显式、有界的孤儿快照清理。它绝不删除未登记的对象、不跟随链接、
/// 不猜测进程是否存活，也不会在保留补丁之前删除工作副本。
pub fn cleanup_orphan_snapshots(
    request: &SnapshotCleanupRequest<'_>,
) -> Result<SnapshotCleanupReport, RunnerError> {
    let source = canonical_directory(request.source_checkout)?;
    let root = canonical_directory(request.attempt_root)?;
    let retained_root = canonical_directory(request.retained_root)?;
    if request.minimum_age_seconds < 300
        || source.starts_with(&root)
        || root.starts_with(&source)
        || retained_root.starts_with(&root)
        || root.starts_with(&retained_root)
        || retained_root.starts_with(&source)
        || source.starts_with(&retained_root)
        || request
            .active_attempts
            .iter()
            .any(|p| p.parent() != Some(root.as_path()))
    {
        return Err(RunnerError::InvalidExecution);
    }
    let lock = root_lock(&root)?;
    {
        let _retained_check = root_lock(&retained_root)?;
    }
    let mut report = SnapshotCleanupReport::default();
    let mut entries = fs::read_dir(&root)
        .map_err(|e| RunnerError::io("orphan snapshot scan", e))?
        .take(4097)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| RunnerError::io("orphan snapshot scan", e))?;
    if entries.len() > 4096 {
        return Err(RunnerError::OutputLimit);
    }
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if entry.file_name() == LOCK || entry.file_name() == CACHE {
            continue;
        }
        let attempt = entry.path();
        if request.active_attempts.contains(&attempt) {
            report.protected.push(attempt);
            continue;
        }
        let record = (|| {
            canonical_directory(&attempt)?;
            let record = read_record::<SnapshotRecord>(&attempt.join(RECORD), lock.owner)?
                .ok_or(RunnerError::InvalidExecution)?;
            if record.schema != 1
                || record.source != source
                || record.source_identity != identity(&source)?
                || record.root_identity != lock.stamp
                || record.attempt_identity != identity(&attempt)?
                || record.repo_identity != identity(&record.repo)?
            {
                return Err(RunnerError::InvalidExecution);
            }
            Ok::<_, RunnerError>(record)
        })();
        let Ok(mut record) = record else {
            report.blocked.push(attempt);
            continue;
        };
        if !request
            .now_unix_seconds
            .checked_sub(record.created_at)
            .is_some_and(|age| age >= request.minimum_age_seconds)
        {
            report.protected.push(attempt);
            continue;
        }
        let result = cleanup_one(&attempt, &retained_root, &mut record, &lock, &mut report);
        if result.is_ok() {
            report.removed.push(attempt);
        } else {
            report.blocked.push(attempt);
        }
    }
    Ok(report)
}
fn cleanup_one(
    attempt: &Path,
    retained: &Path,
    record: &mut SnapshotRecord,
    lock: &RootLock,
    report: &mut SnapshotCleanupReport,
) -> Result<(), RunnerError> {
    let snapshot = CodeSnapshot {
        repo: record.repo.clone(),
        work: attempt.join("work"),
        index: attempt.join("snapshot.index"),
        base: record.base.clone(),
    };
    canonical_directory(&record.repo)?;
    canonical_directory(&record.admin)?;
    validate_tree(attempt)?;
    validate_tree(&record.admin)?;
    if identity(&snapshot.work)? != record.work_identity
        || identity(&record.admin)? != record.admin_identity
    {
        return Err(RunnerError::InvalidExecution);
    }
    let admin_link = fs::read_to_string(record.admin.join("gitdir"))
        .map_err(|e| RunnerError::io("worktree registration", e))?;
    if Path::new(admin_link.trim()) != snapshot.work.join(".git") {
        return Err(RunnerError::InvalidExecution);
    }
    let local_config = git(
        &snapshot.repo,
        &record.source,
        &snapshot.index,
        &["config", "--local", "--list"],
    )?;
    if String::from_utf8_lossy(&local_config).lines().any(|s| {
        s.starts_with("filter.") || s.starts_with("include.") || s.starts_with("includeif.")
    }) {
        return Err(RunnerError::InvalidExecution);
    }
    let name = attempt
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or(RunnerError::InvalidExecution)?;
    let destination = retained.join(format!("{name}-orphan-unlanded.patch"));
    let (metadata, _) = snapshot.preserve(&destination)?;
    record.retained = Some(metadata.clone());
    write_record(&attempt.join(RECORD), record, lock.owner, true)?;
    report.retained.push(metadata);
    // 在准入已停止、scope 已隔离的前提下，删除前再校验一遍剩余的每个节点。
    validate_tree(attempt)?;
    validate_tree(&record.admin)?;
    if identity(attempt)? != record.attempt_identity
        || identity(&record.admin)? != record.admin_identity
        || identity(attempt.parent().ok_or(RunnerError::InvalidExecution)?)? != lock.stamp
    {
        return Err(RunnerError::InvalidExecution);
    }
    fs::remove_dir_all(attempt).map_err(|e| RunnerError::io("remove orphan work copy", e))?;
    fs::remove_dir_all(&record.admin)
        .map_err(|e| RunnerError::io("remove exact worktree registration", e))?;
    sync_directory(attempt.parent().ok_or(RunnerError::InvalidExecution)?)?;
    sync_directory(record.admin.parent().ok_or(RunnerError::InvalidExecution)?)
}

#[cfg(all(test, unix))]
mod lock_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn release_does_not_wait_for_a_duplicated_open_file_description() {
        let temp = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let root = temp.path().canonicalize().unwrap();
        let lease = root_lock(&root).unwrap();
        let inherited = lease._file.try_clone().unwrap();
        assert!(matches!(
            root_lock(&root),
            Err(RunnerError::StateUnavailable)
        ));
        drop(lease);
        let successor = root_lock(&root).unwrap();
        drop(inherited);
        drop(successor);
    }
}
