//! Trusted code-side landing, deliberately absent from RunnerAdapter execution.
use crate::RunnerError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Owner-approved mapping and immutable candidate binding; never deserialize from Runner HTTP.
#[derive(Clone, Debug)]
pub struct CodeLandingPlan {
    /// Administrator-mapped repository checkout.
    pub repository: PathBuf,
    /// Expected HEAD commit, checked before and after applying.
    pub expected_head: String,
    /// Full approved dirty/untracked worktree tree, compared before any application.
    pub expected_tree: String,
    /// Exact candidate result tree; successful application must match this tree.
    pub expected_after_tree: String,
    /// SHA-256 of the already retained candidate.
    pub patch_sha256: String,
    /// Durable unlanded candidate; retained regardless of landing outcome.
    pub patch_path: PathBuf,
}
/// A conflict is an open review item, never a successful task terminal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeLandingResult {
    /// Whether git applied the candidate without conflict markers and exact tree verification.
    pub applied: bool,
    /// Repository-relative paths containing conflict markers.
    pub conflicts: Vec<String>,
    /// Retained candidate always survives failures.
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
    /// Read-only-to-the-worktree precondition check while the caller holds the landing lease.
    pub fn verify_base(&self) -> Result<(), RunnerError> {
        self.head_matches()?;
        let (_, _, current) = private_index(&self.repository)?;
        if current != self.expected_tree {
            return Err(RunnerError::IdempotencyConflict);
        }
        Ok(())
    }
    /// Rechecks a previously recorded landing before replay or owner acceptance.
    pub fn verify_landed(&self) -> Result<(), RunnerError> {
        self.head_matches()?;
        let (_, _, current) = private_index(&self.repository)?;
        if current != self.expected_after_tree {
            return Err(RunnerError::IdempotencyConflict);
        }
        Ok(())
    }
}
/// Applies an explicitly approved patch using a private index for the three-way fallback.
/// Caller holds the repository lease; HEAD, complete dirty tree and candidate bytes are CAS-bound.
/// The user's index, commits and refs are never updated by this operation.
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
/// Compare-and-swap branch advancement for a separately authorized trusted owner only.
/// Candidate landing does not call this function.
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
