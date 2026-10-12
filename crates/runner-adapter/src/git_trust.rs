//! 宿主机自动运行 git 的唯一加固入口：最小环境、惰性配置、本地配置闸门与仓库身份校验。
//!
//! 快照（`snapshot.rs`）与落地（`landing.rs`）共用本模块，免得两处各自演化出不同的信任边界。
//! 威胁模型：被批准的检出目录里的 `.git` 由别人精心构造（解压的压缩包、别人交付的仓库）。
//! 对策分三层：
//! 1. 环境：清空继承的环境变量，全局与系统配置置空，hooks、fsmonitor、外部 diff、分页器、
//!    签名与签名校验一律失效，也不惰性拉取缺失对象、不套用 `refs/replace`；提交者身份固定，
//!    reflog 里不会出现检出配置里写的名字；
//! 2. 本地配置闸门：`-c` 压不住的可执行入口（过滤器、合并驱动、`hook.<名>.command`、include、
//!    部分克隆的 promisor）与改写工作树位置的键，出现就整体拒绝，一个 git 子进程都不再起；
//!    检出里嵌套的仓库会被外层的 `add -A`／`diff` 拉起一个读它自己配置的 git（登记成 gitlink
//!    的、未登记但会被收进私有 index 的、补丁新加 gitlink 的都会），所以按文件系统找全、逐个审计；
//! 3. 身份：`.git` 必须真属于这个检出目录且属主是当前用户——普通克隆是目录里的真 `.git`，
//!    链接工作树要有反向指针，外来的 `gitdir:` 指针、`commondir` 重定向、`.git` 里指向别处的
//!    符号链接一律拒绝；`refs/`、`logs/` 整棵子树里不许有符号链接与硬链接（引用与 reflog
//!    都写在这里）。
//!
//! 配置闸门与嵌套仓库审计在子模块 `audit`。
//!
//! 残留风险：检查与随后的 git 子进程之间有一个窗口，同检出里并发运行的进程可以在其中改
//! `.git`；快照与落地在最危险的写操作之前会再审计一次以缩小窗口，但不能消除。
use crate::RunnerError;
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

mod audit;
pub(crate) use audit::audit;

/// git 管理目录里 git 会顺着走的条目：它们若是符号链接，配置读取或引用写入就会落到仓库之外。
/// `objects`、`hooks`、`info` 不在其列：有人会特意把它们链出去，而 hooks 本来就被关掉了。
const PLAIN_ENTRIES: [&str; 10] = [
    "config",
    "config.worktree",
    "commondir",
    "gitdir",
    "HEAD",
    "index",
    "packed-refs",
    "refs",
    "logs",
    "worktrees",
];

/// 引用与 reflog 所在的子树，里面每一层都要查：`update-ref` 顺着子目录写引用、以追加方式写
/// reflog，`refs/heads` 链到别的仓库就改了那个仓库的分支，reflog 文件链到仓库外就往那个文件
/// 追加一行（2026-10-11 在 git 2.53 实测两种都成立）。
const PLAIN_TREES: [&str; 2] = ["refs", "logs"];

/// 身份校验通过的仓库：三个路径都是规范绝对路径，且与 git 自己的报告一致。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitRepo {
    /// 工作树顶层，即调用方批准的检出目录本身。
    pub work: PathBuf,
    /// 该检出的管理目录：普通克隆是 `<work>/.git`，链接工作树是 `<common>/worktrees/<名>`。
    pub git_dir: PathBuf,
    /// 对象与引用所在的公共目录；普通克隆下与 `git_dir` 相同。
    pub common_dir: PathBuf,
}

/// 全部自动 git 共用的命令底座：不继承任何环境变量，且让可执行入口失效。
pub(crate) fn command() -> Command {
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        // 缺失的对象不去惰性拉取（那会执行 promisor 远端自带的上传程序），
        // `refs/replace` 也不能改写提交的内容。
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        // 引用更新会往 reflog 追加一行提交者身份，不给就取仓库本地配置里别人写的 `user.name`；
        // 固定下来，reflog 的内容就不受检出控制。快照另设自己的身份，会覆盖这里。
        .env("GIT_AUTHOR_NAME", "Studiolo runner")
        .env("GIT_AUTHOR_EMAIL", "runner@invalid")
        .env("GIT_COMMITTER_NAME", "Studiolo runner")
        .env("GIT_COMMITTER_EMAIL", "runner@invalid")
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
            // 本地配置开了 `log.showSignature` 时，`git show` 遇到带签名头的提交会执行
            // `gpg.program`（2026-10-11 在 git 2.53 实测）；校验签名对快照毫无用处，一律关掉。
            "-c",
            "log.showSignature=false",
            // 快照的 `commit-tree` 若按本地 `commit.gpgSign` 签名，就会执行本地配置的签名程序。
            // git 2.10 起 `commit-tree` 已不读这个键（2026-10-11 在 git 2.53 实测 openpgp 与 ssh
            // 两种格式都不执行）；照样压掉，保证不取决于宿主机的 git 版本。
            "-c",
            "commit.gpgSign=false",
        ])
        .stdin(Stdio::null());
    command
}

/// 显式指定管理目录与工作树的命令，绝不让 git 自己「发现」仓库。
/// 快照的沙箱副本不是 `<work>/.git` 布局，所以这里不要求身份已校验；
/// 需要校验的调用方先走 [`open`]。
pub(crate) fn scoped(git_dir: &Path, work: &Path, index: Option<&Path>) -> Command {
    let mut command = command();
    command
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(work);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command
}

impl GitRepo {
    /// 在已校验的仓库上起命令；工作目录就是工作树顶层，与原先的 `git -C <仓库>` 一致。
    pub(crate) fn git(&self, index: Option<&Path>) -> Command {
        let mut command = scoped(&self.git_dir, &self.work, index);
        command.current_dir(&self.work);
        command
    }

    /// 重新审计本仓库的配置与嵌套仓库；在最危险的写操作之前再调一次，缩小检查到使用的窗口。
    pub(crate) fn audit(&self) -> Result<(), RunnerError> {
        audit(&self.git_dir, &self.work)
    }
}

fn capture(git_dir: &Path, work: &Path, args: &[&str]) -> Result<Vec<u8>, RunnerError> {
    let output = scoped(git_dir, work, None)
        .current_dir(work)
        .args(args)
        .output()
        .map_err(|_| RunnerError::InvalidExecution)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(RunnerError::InvalidExecution)
    }
}

/// 当前进程的有效 uid。不引入新依赖：新建的匿名临时文件属于创建它的有效用户。
fn current_uid() -> Result<u32, RunnerError> {
    tempfile::tempfile()
        .and_then(|file| file.metadata())
        .map(|meta| meta.uid())
        .map_err(|_| RunnerError::InvalidExecution)
}

/// 打开并校验 `repo`；每次操作都重新调用、绝不缓存，之后任何替换、重指向或配置变化都会被拒绝。
pub(crate) fn open(repo: &Path) -> Result<GitRepo, RunnerError> {
    let work = fs::canonicalize(repo).map_err(|_| RunnerError::InvalidExecution)?;
    let (git_dir, common_dir) = admin_dirs(&work)?;
    // 让 git 自己再报告一遍：它与上面推出的身份不一致，就说明有我们没读懂的重定向。
    let report = capture(
        &git_dir,
        &work,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--absolute-git-dir",
            "--git-common-dir",
            "--show-toplevel",
        ],
    )?;
    let report = String::from_utf8(report).map_err(|_| RunnerError::InvalidExecution)?;
    let lines: Vec<&str> = report.lines().collect();
    if lines.len() != 3 {
        return Err(RunnerError::InvalidExecution);
    }
    for (reported, expected) in lines.iter().zip([&git_dir, &common_dir, &work]) {
        if fs::canonicalize(reported).ok().as_deref() != Some(expected.as_path()) {
            return Err(RunnerError::InvalidExecution);
        }
    }
    audit(&git_dir, &work)?;
    Ok(GitRepo {
        work,
        git_dir,
        common_dir,
    })
}

/// 只读取检出目录自己的 `.git`，不问 git：普通克隆与链接工作树之外的布局一律拒绝。
fn admin_dirs(work: &Path) -> Result<(PathBuf, PathBuf), RunnerError> {
    let invalid = || RunnerError::InvalidExecution;
    let dotgit = work.join(".git");
    let meta = fs::symlink_metadata(&dotgit).map_err(|_| invalid())?;
    let (git_dir, common_dir) = if meta.is_dir() {
        // 普通克隆。管理目录里有 `commondir` 就是自称别处仓库的链接工作树，公共目录会被改写。
        let git_dir = fs::canonicalize(&dotgit).map_err(|_| invalid())?;
        if git_dir != dotgit || fs::symlink_metadata(git_dir.join("commondir")).is_ok() {
            return Err(invalid());
        }
        (git_dir.clone(), git_dir)
    } else {
        if !meta.is_file() || meta.len() > 4096 {
            return Err(invalid());
        }
        // 链接工作树：`.git` 是一行 `gitdir: <管理目录>`，合法的管理目录必须反过来指回本检出。
        let text = fs::read_to_string(&dotgit).map_err(|_| invalid())?;
        let target = text
            .trim_end()
            .strip_prefix("gitdir: ")
            .ok_or_else(invalid)?;
        let target = Path::new(target);
        let target = if target.is_absolute() {
            target.to_path_buf()
        } else {
            work.join(target)
        };
        let git_dir = fs::canonicalize(target).map_err(|_| invalid())?;
        let common = fs::read_to_string(git_dir.join("commondir")).map_err(|_| invalid())?;
        let common_dir =
            fs::canonicalize(git_dir.join(common.trim_end())).map_err(|_| invalid())?;
        if git_dir.parent() != Some(common_dir.join("worktrees").as_path()) {
            return Err(invalid());
        }
        // 反向指针可能是相对路径（`worktree.useRelativePaths`），相对管理目录解析。
        let back = fs::read_to_string(git_dir.join("gitdir")).map_err(|_| invalid())?;
        let back = git_dir.join(back.trim_end());
        if fs::canonicalize(back).ok().as_deref() != Some(dotgit.as_path()) {
            return Err(invalid());
        }
        (git_dir, common_dir)
    };
    // 显式 `--git-dir` 会让 git 跳过它自己的属主检查（`safe.directory`），所以在这里补上；
    // 并且 git 顺着走的条目不得是符号链接。
    let uid = current_uid()?;
    for folder in [
        work,
        dotgit.as_path(),
        git_dir.as_path(),
        common_dir.as_path(),
    ] {
        require_owned(folder, uid)?;
    }
    require_plain_entries(&[git_dir.as_path(), common_dir.as_path()])?;
    Ok((git_dir, common_dir))
}

fn require_owned(path: &Path, uid: u32) -> Result<(), RunnerError> {
    let meta = fs::symlink_metadata(path).map_err(|_| RunnerError::InvalidExecution)?;
    if meta.uid() == uid {
        Ok(())
    } else {
        Err(RunnerError::InvalidExecution)
    }
}

fn require_plain_entries(git_dirs: &[&Path]) -> Result<(), RunnerError> {
    for git_dir in git_dirs {
        for name in PLAIN_ENTRIES {
            if let Ok(meta) = fs::symlink_metadata(git_dir.join(name))
                && meta.file_type().is_symlink()
            {
                return Err(RunnerError::InvalidExecution);
            }
        }
        for name in PLAIN_TREES {
            require_plain_tree(&git_dir.join(name))?;
        }
    }
    Ok(())
}

/// 子树里只许有真目录和只有一个链接的普通文件：符号链接把写入带出仓库，硬链接让追加进 reflog
/// 的那一行同时写进别处的同一个文件。子树不存在不算错（新仓库还没有 `logs`）。
fn require_plain_tree(root: &Path) -> Result<(), RunnerError> {
    let invalid = || RunnerError::InvalidExecution;
    match fs::symlink_metadata(root) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(invalid()),
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).map_err(|_| invalid())? {
            let entry = entry.map_err(|_| invalid())?;
            // `DirEntry::metadata` 不跟随符号链接。
            let meta = entry.metadata().map_err(|_| invalid())?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else if !meta.is_file() || meta.nlink() != 1 {
                return Err(invalid());
            }
        }
    }
    Ok(())
}
