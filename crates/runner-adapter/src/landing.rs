//! 可信的代码侧落地；刻意不出现在 RunnerAdapter 的执行路径中。
use crate::RunnerError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// 经所有者批准的映射与不可变的候选绑定；绝不从 Runner 的 HTTP 请求反序列化。
#[derive(Clone, Debug)]
pub struct CodeLandingPlan {
    /// 管理员映射的仓库检出目录。
    pub repository: PathBuf,
    /// 预期的 HEAD 提交，应用前后都会检查。
    pub expected_head: String,
    /// 经批准的完整工作树 tree（含未提交与未跟踪内容），在任何应用之前比对。
    pub expected_tree: String,
    /// 精确的候选结果 tree；应用成功后必须与之一致。
    pub expected_after_tree: String,
    /// 已保留候选补丁的 SHA-256。
    pub patch_sha256: String,
    /// 持久化的未落地候选补丁；无论落地结果如何都会保留。
    pub patch_path: PathBuf,
}
/// 冲突是待处理的评审项，绝不算任务的成功终态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeLandingResult {
    /// git 是否成功应用候选补丁：没有冲突标记，且通过精确的 tree 校验。
    pub applied: bool,
    /// 含冲突标记或处于未合并状态的文件路径（相对仓库根）。
    pub conflicts: Vec<String>,
    /// 已保留的候选补丁在任何失败下都会留存。
    pub unlanded_patch: PathBuf,
}
fn git(
    repo: &Path,
    index: Option<&Path>,
    args: &[&str],
) -> Result<std::process::Output, RunnerError> {
    let mut command = Command::new("git");
    command
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "diff.external=",
        ])
        .args(args)
        .stdin(Stdio::null());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    command.output().map_err(|_| RunnerError::InvalidExecution)
}
fn ok(repo: &Path, index: Option<&Path>, args: &[&str]) -> Result<Vec<u8>, RunnerError> {
    let result = git(repo, index, args)?;
    if result.status.success() {
        Ok(result.stdout)
    } else {
        Err(RunnerError::InvalidExecution)
    }
}
fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn text_oid(bytes: Vec<u8>) -> Result<String, RunnerError> {
    let value = String::from_utf8(bytes)
        .map_err(|_| RunnerError::InvalidExecution)?
        .trim()
        .to_owned();
    if oid(&value) {
        Ok(value)
    } else {
        Err(RunnerError::InvalidExecution)
    }
}
fn private_index(repo: &Path) -> Result<(tempfile::TempDir, PathBuf, String), RunnerError> {
    use std::os::unix::fs::PermissionsExt;
    let config = ok(repo, None, &["config", "--local", "--list"])?;
    if String::from_utf8_lossy(&config).lines().any(|line| {
        line.starts_with("filter.")
            || line.starts_with("include.")
            || line.starts_with("includeif.")
            || line.starts_with("extensions.worktreeconfig=")
    }) {
        return Err(RunnerError::InvalidExecution);
    }
    let directory = tempfile::Builder::new()
        .prefix("studiolo-landing-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir()
        .map_err(|_| RunnerError::InvalidExecution)?;
    let index = directory.path().join("index");
    ok(repo, Some(&index), &["read-tree", "HEAD"])?;
    ok(repo, Some(&index), &["add", "-A", "--", "."])?;
    let tree = text_oid(ok(repo, Some(&index), &["write-tree"])?)?;
    Ok((directory, index, tree))
}
impl CodeLandingPlan {
    fn head_matches(&self) -> Result<(), RunnerError> {
        if !oid(&self.expected_head)
            || !oid(&self.expected_tree)
            || !oid(&self.expected_after_tree)
            || text_oid(ok(&self.repository, None, &["rev-parse", "HEAD"])?)? != self.expected_head
        {
            return Err(RunnerError::IdempotencyConflict);
        }
        Ok(())
    }
    /// 前置条件检查（不改动工作树）；调用方须持有落地租约。
    pub fn verify_base(&self) -> Result<(), RunnerError> {
        self.head_matches()?;
        let (_, _, current) = private_index(&self.repository)?;
        if current != self.expected_tree {
            return Err(RunnerError::IdempotencyConflict);
        }
        Ok(())
    }
    /// 在重放或所有者验收之前，复核先前记录的落地结果。
    pub fn verify_landed(&self) -> Result<(), RunnerError> {
        self.head_matches()?;
        let (_, _, current) = private_index(&self.repository)?;
        if current != self.expected_after_tree {
            return Err(RunnerError::IdempotencyConflict);
        }
        Ok(())
    }
}
/// 应用一个经显式批准的补丁；三方合并回退使用私有 index。
/// 调用方须持有仓库租约；HEAD、完整的脏工作树与候选字节都做 CAS 绑定。
/// 本操作绝不更新用户的 index、提交或 ref。
pub fn apply_code_patch(plan: &CodeLandingPlan) -> Result<CodeLandingResult, RunnerError> {
    let metadata = fs::symlink_metadata(&plan.patch_path)
        .map_err(|e| RunnerError::io("candidate metadata", e))?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err(RunnerError::InvalidExecution);
    }
    let bytes =
        fs::read(&plan.patch_path).map_err(|e| RunnerError::io("read retained candidate", e))?;
    if bytes.len() > 16 * 1024 * 1024
        || Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            != plan.patch_sha256
    {
        return Err(RunnerError::InvalidExecution);
    }
    plan.head_matches()?;
    if !ok(
        &plan.repository,
        None,
        &["diff", "--name-only", "--diff-filter=U", "-z"],
    )?
    .is_empty()
    {
        return Err(RunnerError::IdempotencyConflict);
    }
    let (_private, index, before) = private_index(&plan.repository)?;
    if before != plan.expected_tree {
        return Err(RunnerError::IdempotencyConflict);
    }
    if bytes.is_empty() && plan.expected_tree == plan.expected_after_tree {
        return Ok(CodeLandingResult {
            applied: plan.verify_landed().is_ok(),
            conflicts: Vec::new(),
            unlanded_patch: plan.patch_path.clone(),
        });
    }
    let path = plan
        .patch_path
        .to_str()
        .ok_or(RunnerError::InvalidExecution)?;
    let check = git(&plan.repository, None, &["apply", "--check", "--", path])?;
    let applied = if check.status.success() {
        git(&plan.repository, None, &["apply", "--", path])?
    } else {
        git(
            &plan.repository,
            Some(&index),
            &["apply", "--3way", "--", path],
        )?
    };
    let unmerged = ok(
        &plan.repository,
        Some(&index),
        &["diff", "--name-only", "--diff-filter=U", "-z"],
    )?;
    let mut conflicts: Vec<String> = unmerged
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect();
    let tracked = ok(&plan.repository, Some(&index), &["ls-files", "-z"])?;
    for path in tracked.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(path).map_err(|_| RunnerError::InvalidExecution)?;
        let full = plan.repository.join(path);
        let metadata = match full.symlink_metadata() {
            Ok(value) => value,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(RunnerError::io("landing file metadata", e)),
        };
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > 16 * 1024 * 1024 {
            return Err(RunnerError::InvalidExecution);
        }
        let bytes = fs::read(full).map_err(|e| RunnerError::io("landing marker scan", e))?;
        if bytes
            .split(|b| *b == b'\n')
            .any(|l| l.starts_with(b"<<<<<<< ") || l.starts_with(b">>>>>>> "))
        {
            conflicts.push(path.to_owned());
        }
    }
    conflicts.sort();
    conflicts.dedup();
    Ok(CodeLandingResult {
        applied: applied.status.success() && conflicts.is_empty() && plan.verify_landed().is_ok(),
        conflicts,
        unlanded_patch: plan.patch_path.clone(),
    })
}
/// 以 CAS 方式推进分支，仅供另行授权的可信所有者使用。
/// 候选落地流程不会调用此函数。
pub fn advance_code_ref(
    repository: &Path,
    reference: &str,
    new_commit: &str,
    expected_old: &str,
) -> Result<(), RunnerError> {
    if !reference.starts_with("refs/heads/") || !oid(new_commit) || !oid(expected_old) {
        return Err(RunnerError::InvalidExecution);
    }
    if git(
        repository,
        None,
        &["update-ref", reference, new_commit, expected_old],
    )?
    .status
    .success()
    {
        Ok(())
    } else {
        Err(RunnerError::IdempotencyConflict)
    }
}
