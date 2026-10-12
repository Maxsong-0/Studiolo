//! 本地配置闸门与嵌套仓库审计：见上级模块的威胁模型。
use super::{capture, current_uid, require_owned, require_plain_entries};
use crate::RunnerError;
use std::{
    ffi::OsStr,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

/// 单个检出里最多审计的嵌套仓库数；再多就拒绝，不做半截检查。
const NESTED_LIMIT: usize = 64;

/// 审计仓库本地配置（含 `config.worktree`）与检出里全部嵌套仓库的配置；全局与系统配置已被置空，
/// 列出来的只剩仓库自己的。
pub(crate) fn audit(git_dir: &Path, work: &Path) -> Result<(), RunnerError> {
    audit_config(git_dir, work, false)?;
    audit_nested(git_dir, work)
}

fn audit_config(git_dir: &Path, work: &Path, nested: bool) -> Result<(), RunnerError> {
    let listing = capture(git_dir, work, &["config", "--no-includes", "--list", "-z"])?;
    if forbidden_config_key(&listing, nested).is_some() {
        return Err(RunnerError::InvalidExecution);
    }
    if nested {
        // 子模块靠 `core.worktree` 连回自己的文件夹，那是正常的；指到别处，嵌套的 git 就会去扫
        // 一棵没审过的树（那里的 gitlink 与属性都不在本检出里）。相对路径按 git 的规矩相对管理目录。
        for value in config_values(&listing, "core.worktree") {
            let target = fs::canonicalize(git_dir.join(OsStr::from_bytes(value)))
                .map_err(|_| RunnerError::InvalidExecution)?;
            if target != work {
                return Err(RunnerError::InvalidExecution);
            }
        }
    }
    Ok(())
}

/// 检出里每个带 `.git` 条目的目录都按嵌套仓库审计。外层的 `add -A`、`diff` 只要把那个路径当成
/// gitlink——HEAD 或暂存区里已有的、第一次 `add -A` 把未登记的嵌套仓库收进私有 index 的、补丁经
/// `apply --3way` 新加的——就会在里面起一个读它自己配置与属性的 git（2026-10-11 在 git 2.53 实测
/// 后两种都会跑它的过滤器）。「是否已登记」挡不住后两种，所以不看登记，按文件系统找全；
/// 与 git 一样不跟随符号链接，也不进任何 `.git`。
fn audit_nested(git_dir: &Path, work: &Path) -> Result<(), RunnerError> {
    let common = common_dir_of(git_dir)?;
    let layout = Layout {
        // 子模块的管理目录住在外层这个检出自己的 `modules/` 下（链接工作树也是各管各的）。
        modules: git_dir.join("modules"),
        worktrees: common.join("worktrees"),
        common,
    };
    let mut audited = 0;
    let mut pending = vec![work.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let mut is_repository = false;
        for entry in fs::read_dir(&folder).map_err(|_| RunnerError::InvalidExecution)? {
            let entry = entry.map_err(|_| RunnerError::InvalidExecution)?;
            if entry.file_name() == ".git" {
                is_repository = true;
            } else if entry
                .file_type()
                .map_err(|_| RunnerError::InvalidExecution)?
                .is_dir()
            {
                pending.push(entry.path());
            }
        }
        if !is_repository || folder == work {
            continue;
        }
        audited += 1;
        if audited > NESTED_LIMIT {
            return Err(RunnerError::InvalidExecution);
        }
        let nested_git = nested_git_dir(&folder, &layout)?;
        audit_config(&nested_git, &folder, true)?;
    }
    Ok(())
}

/// 外层仓库里嵌套仓库的管理目录可以合法落脚的位置。
struct Layout {
    /// 外层这个检出的 `modules/`：子模块的管理目录。
    modules: PathBuf,
    /// 外层公共目录的 `worktrees/`：同仓库链接工作树的管理目录。
    worktrees: PathBuf,
    /// 外层公共目录。
    common: PathBuf,
}

/// 管理目录对应的公共目录：链接工作树的管理目录里 `commondir` 指过去，普通仓库就是它自己。
fn common_dir_of(git_dir: &Path) -> Result<PathBuf, RunnerError> {
    match fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => fs::canonicalize(git_dir.join(text.trim_end()))
            .map_err(|_| RunnerError::InvalidExecution),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(git_dir.to_path_buf()),
        Err(_) => Err(RunnerError::InvalidExecution),
    }
}

/// 嵌套仓库的管理目录只认三种布局，其余（指到另一个仓库、另一个检出）一律拒绝——外层 git 在
/// 里面起的子进程会改那个仓库的 index：
/// 1. `.git` 是文件夹里的真目录（内嵌的独立克隆），没有 `commondir`；
/// 2. `gitdir:` 指到外层 `modules/` 下（子模块），没有 `commondir`；
/// 3. `gitdir:` 指到本仓库公共目录的 `worktrees/` 下（放在检出里的同仓库链接工作树，例如
///    `.claude/worktrees/`），其 `commondir` 指回本仓库、`gitdir` 反向指针指回这个文件夹。
///    它自己的 `config.worktree` 由随后的配置审计一并列出。
///
/// 属主须是当前用户，管理目录里没有链接条目。
fn nested_git_dir(folder: &Path, layout: &Layout) -> Result<PathBuf, RunnerError> {
    let invalid = || RunnerError::InvalidExecution;
    let dotgit = folder.join(".git");
    let meta = fs::symlink_metadata(&dotgit).map_err(|_| invalid())?;
    let (git_dir, linked) = if meta.is_dir() {
        let git_dir = fs::canonicalize(&dotgit).map_err(|_| invalid())?;
        if git_dir != dotgit {
            return Err(invalid());
        }
        (git_dir, false)
    } else if meta.is_file() && meta.len() <= 4096 {
        let text = fs::read_to_string(&dotgit).map_err(|_| invalid())?;
        let target = text
            .trim_end()
            .strip_prefix("gitdir: ")
            .ok_or_else(invalid)?;
        let git_dir = fs::canonicalize(folder.join(target)).map_err(|_| invalid())?;
        if !git_dir.is_dir() {
            return Err(invalid());
        }
        if git_dir != layout.modules && git_dir.starts_with(&layout.modules) {
            (git_dir, false)
        } else if git_dir.parent() == Some(layout.worktrees.as_path()) {
            (git_dir, true)
        } else {
            return Err(invalid());
        }
    } else {
        return Err(invalid());
    };
    if linked {
        if common_dir_of(&git_dir)? != layout.common {
            return Err(invalid());
        }
        let back = fs::read_to_string(git_dir.join("gitdir")).map_err(|_| invalid())?;
        if fs::canonicalize(git_dir.join(back.trim_end()))
            .ok()
            .as_deref()
            != Some(dotgit.as_path())
        {
            return Err(invalid());
        }
    } else if fs::symlink_metadata(git_dir.join("commondir")).is_ok() {
        return Err(invalid());
    }
    let uid = current_uid()?;
    require_owned(folder, uid)?;
    require_owned(&git_dir, uid)?;
    require_plain_entries(&[git_dir.as_path()])?;
    Ok(git_dir)
}

/// 把 `git config --list -z` 的输出拆成（键，值）；条目形如 `键\n值\0`，没有值的写成 `键\0`。
fn config_entries(listing: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    listing
        .split(|b| *b == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| match entry.iter().position(|b| *b == b'\n') {
            Some(at) => (&entry[..at], &entry[at + 1..]),
            None => (entry, &entry[entry.len()..]),
        })
}

/// 某个键（大小写不敏感）的全部取值，没有值的条目记作空串；纯函数，便于单测。
fn config_values<'a>(listing: &'a [u8], key: &str) -> Vec<&'a [u8]> {
    config_entries(listing)
        .filter(|(name, _)| name.eq_ignore_ascii_case(key.as_bytes()))
        .map(|(_, value)| value)
        .collect()
}

/// 在 `git config --list -z` 的输出里找第一个禁用键；纯函数，便于单测。只看键，值里长得像
/// 禁用键的文字不算。`nested` 是嵌套仓库：子模块靠 `core.worktree` 与自己的文件夹相连，
/// 这里放行这个键，取值另由 [`audit_config`] 核对必须指回该文件夹。
pub(crate) fn forbidden_config_key(listing: &[u8], nested: bool) -> Option<String> {
    config_entries(listing)
        .map(|(key, _)| String::from_utf8_lossy(key).to_ascii_lowercase())
        .find(|key| is_forbidden(key, nested))
}

fn is_forbidden(key: &str, nested: bool) -> bool {
    // 过滤器（add／status 时执行 clean）、合并驱动（`apply --3way` 时执行）、配置式钩子
    // （`hook.<名>.command`，`core.hooksPath` 管不到）与 include 都能跑宿主机代码，且 `-c`
    // 无法逐个压掉；部分克隆的 promisor 会在缺对象时执行远端自带的上传程序；
    // `core.worktree` 把工作树改指别处。diff 驱动的 textconv／command 今天的调用都用
    // `--no-textconv`／`--no-ext-diff` 或 `--name-only` 避开了，同样拒绝，免得日后新增的 diff 踩上。
    let promisor = key.starts_with("remote.")
        && (key.ends_with(".promisor") || key.ends_with(".partialclonefilter"));
    let diff_driver =
        key.starts_with("diff.") && (key.ends_with(".textconv") || key.ends_with(".command"));
    key.starts_with("filter.")
        || key.starts_with("include.")
        || key.starts_with("includeif.")
        || key.starts_with("hook.")
        || (key.starts_with("merge.") && key.ends_with(".driver"))
        || (key == "core.worktree" && !nested)
        || key == "extensions.partialclone"
        || key == "core.partialclonefilter"
        || promisor
        || diff_driver
}

#[cfg(test)]
mod tests {
    use super::{config_values, forbidden_config_key};

    #[test]
    fn code_execution_and_location_keys_are_forbidden() {
        for entry in [
            "filter.lfs.clean\ngit-lfs clean\0",
            "filter.evil.process\nx\0",
            "merge.evil.driver\ntouch marker\0",
            "merge.my.evil.driver\nx\0",
            "hook.x.command\ntouch marker\0",
            "hook.x.event\npost-index-change\0",
            "include.path\n/tmp/x\0",
            "includeIf.gitdir:/x/.path\ny\0",
            "core.worktree\n/elsewhere\0",
            "core.worktree\0",
            "extensions.partialClone\norigin\0",
            "core.partialCloneFilter\nblob:none\0",
            "remote.origin.promisor\ntrue\0",
            "remote.origin.partialclonefilter\nblob:none\0",
            "diff.evil.textconv\ntouch marker\0",
            "diff.evil.command\ntouch marker\0",
        ] {
            let listing = format!("core.filemode\ntrue\0{entry}core.bare\nfalse\0");
            assert!(
                forbidden_config_key(listing.as_bytes(), false).is_some(),
                "{entry:?}"
            );
        }
    }
    #[test]
    fn ordinary_and_already_neutralised_keys_pass() {
        let listing = "core.filemode\ntrue\0core.hookspath\n/dev/null\0core.fsmonitor\nfalse\0\
            merge.ff\nonly\0merge.evil.name\nlabel\0user.name\nn\0diff.external\n\0\
            extensions.worktreeconfig\ntrue\0remote.origin.url\nhttps://x/y\0";
        assert_eq!(forbidden_config_key(listing.as_bytes(), false), None);
    }
    #[test]
    fn a_value_that_looks_like_a_forbidden_key_is_not_a_key() {
        let listing = "alias.x\nfilter.evil.clean=1\0user.name\nmerge.a.driver\0";
        assert_eq!(forbidden_config_key(listing.as_bytes(), false), None);
    }
    #[test]
    fn a_submodules_core_worktree_key_passes_but_nothing_else_is_relaxed() {
        let worktree = "core.worktree\n../../../sub\0";
        assert_eq!(forbidden_config_key(worktree.as_bytes(), true), None);
        let filter = "core.worktree\n../../../sub\0filter.x.clean\ncat\0";
        assert!(forbidden_config_key(filter.as_bytes(), true).is_some());
    }
    #[test]
    fn every_value_of_a_key_is_reported_including_empty_ones() {
        let listing =
            b"core.bare\nfalse\0core.worktree\n../../../sub\0Core.WorkTree\0user.name\nn\0";
        let values = config_values(listing, "core.worktree");
        assert_eq!(values, [&b"../../../sub"[..], &b""[..]]);
        assert!(config_values(listing, "core.hookspath").is_empty());
    }
}
