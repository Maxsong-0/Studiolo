//! 自动 git 的信任边界：检出目录里别人构造的 `.git`（可执行的本地配置、外来 gitdir 指针、
//! commondir 重定向、改写工作树位置）必须在任何 git 写操作之前被拒绝，合法布局照常通过。
#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
};
use studiolo_runner_adapter::{
    CodeLandingPlan, CodeSnapshot, RunnerError, advance_code_ref, apply_code_patch,
};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn init_repository(path: &Path) {
    fs::create_dir(path).unwrap();
    git(path, &["init", "-q"]);
    fs::write(path.join("file.txt"), "baseline\n").unwrap();
    git(path, &["add", "."]);
    git(
        path,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"],
    );
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    attempts: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source");
        let attempts = root.join("attempts");
        init_repository(&source);
        fs::create_dir(&attempts).unwrap();
        fs::set_permissions(&attempts, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            _temp: temp,
            root,
            source,
            attempts,
        }
    }
    fn attempt(&self, name: &str) -> PathBuf {
        let path = self.attempts.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
    /// 补丁内容无关紧要：闸门必须在第一个 git 子进程之前就拒绝。
    fn landing_plan(&self) -> CodeLandingPlan {
        let patch = self.root.join("candidate.patch");
        let bytes = b"not a real patch\n";
        fs::write(&patch, bytes).unwrap();
        CodeLandingPlan {
            repository: self.source.clone(),
            expected_head: "0".repeat(40),
            expected_tree: "0".repeat(40),
            expected_after_tree: "0".repeat(40),
            patch_sha256: Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            patch_path: patch,
        }
    }
    fn assert_refused(&self, name: &str) {
        assert!(matches!(
            CodeSnapshot::create(&self.source, &self.attempt(name)),
            Err(RunnerError::InvalidExecution)
        ));
        assert!(matches!(
            apply_code_patch(&self.landing_plan()),
            Err(RunnerError::InvalidExecution)
        ));
    }
}

#[test]
fn merge_driver_config_is_refused_before_the_driver_can_run() {
    let f = Fixture::new();
    let marker = f.root.join("DRIVER_RAN");
    fs::write(f.source.join(".gitattributes"), "* merge=evil\n").unwrap();
    git(&f.source, &["add", ".gitattributes"]);
    git(
        &f.source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "attributes"],
    );
    let driver = format!("touch {}; exit 0", marker.display());
    git(&f.source, &["config", "merge.evil.driver", &driver]);
    f.assert_refused("merge-driver");
    assert!(!marker.exists());
}
#[test]
fn clean_filter_config_is_refused_before_the_filter_can_run() {
    let f = Fixture::new();
    let marker = f.root.join("FILTER_RAN");
    fs::write(f.source.join(".gitattributes"), "* filter=evil\n").unwrap();
    let filter = format!("touch {}; cat", marker.display());
    git(&f.source, &["config", "filter.evil.clean", &filter]);
    f.assert_refused("clean-filter");
    assert!(!marker.exists());
}
#[test]
fn foreign_gitdir_pointer_is_refused_and_the_foreign_repository_is_untouched() {
    let f = Fixture::new();
    let foreign = f.root.join("foreign");
    init_repository(&foreign);
    let before = git(&foreign, &["count-objects", "-v"]);
    fs::remove_dir_all(f.source.join(".git")).unwrap();
    fs::write(
        f.source.join(".git"),
        format!("gitdir: {}\n", foreign.join(".git").display()),
    )
    .unwrap();
    f.assert_refused("foreign-pointer");
    assert!(!foreign.join(".git/worktrees").exists());
    assert_eq!(before, git(&foreign, &["count-objects", "-v"]));
}
#[test]
fn commondir_redirect_is_refused_and_the_foreign_repository_is_untouched() {
    let f = Fixture::new();
    let foreign = f.root.join("foreign");
    init_repository(&foreign);
    let before = git(&foreign, &["count-objects", "-v"]);
    fs::write(
        f.source.join(".git/commondir"),
        format!("{}\n", foreign.join(".git").display()),
    )
    .unwrap();
    f.assert_refused("commondir");
    assert!(!foreign.join(".git/worktrees").exists());
    assert_eq!(before, git(&foreign, &["count-objects", "-v"]));
}
#[test]
fn core_worktree_redirect_is_refused() {
    let f = Fixture::new();
    let elsewhere = f.root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    git(
        &f.source,
        &["config", "core.worktree", elsewhere.to_str().unwrap()],
    );
    f.assert_refused("core-worktree");
}
#[test]
fn a_linked_worktree_checkout_is_still_accepted() {
    let f = Fixture::new();
    let linked = f.root.join("linked");
    git(
        &f.source,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            linked.to_str().unwrap(),
        ],
    );
    let snapshot = CodeSnapshot::create(&linked, &f.attempt("linked")).unwrap();
    assert!(snapshot.work.exists());
}
#[test]
fn configured_hooks_are_refused_before_they_can_run() {
    let f = Fixture::new();
    let marker = f.root.join("HOOK_RAN");
    git(
        &f.source,
        &["config", "hook.evil.event", "post-index-change"],
    );
    let command = format!("touch {}", marker.display());
    git(&f.source, &["config", "hook.evil.command", &command]);
    f.assert_refused("configured-hook");
    assert!(!marker.exists());
}
#[test]
fn a_partial_clone_promisor_is_refused_before_a_missing_object_can_run_its_remote() {
    let f = Fixture::new();
    let marker = f.root.join("UPLOADPACK_RAN");
    git(&f.source, &["config", "extensions.partialclone", "origin"]);
    git(&f.source, &["config", "remote.origin.promisor", "true"]);
    let program = format!("touch {}; exit 1", marker.display());
    git(&f.source, &["config", "remote.origin.uploadpack", &program]);
    f.assert_refused("promisor");
    assert!(!marker.exists());
}
#[test]
fn a_symbolic_link_in_the_git_directory_is_refused() {
    let f = Fixture::new();
    let real = f.root.join("config-elsewhere");
    fs::rename(f.source.join(".git/config"), &real).unwrap();
    symlink(&real, f.source.join(".git/config")).unwrap();
    f.assert_refused("linked-config");
}
/// 嵌套仓库（已登记 gitlink，目录里自带 `.git`）：外层的 `add -A` 会在里面起一个 git，
/// 所以它自己的配置也得过审。
fn add_nested_repository(f: &Fixture) -> PathBuf {
    let nested = f.source.join("vendor");
    init_repository(&nested);
    git(&f.source, &["add", "vendor"]);
    git(
        &f.source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "nested"],
    );
    nested
}
#[test]
fn a_nested_repositorys_filter_is_refused_before_the_outer_add_can_start_it() {
    let f = Fixture::new();
    let nested = add_nested_repository(&f);
    let marker = f.root.join("NESTED_FILTER_RAN");
    fs::write(nested.join(".gitattributes"), "* filter=evil\n").unwrap();
    let filter = format!("touch {}; cat", marker.display());
    git(&nested, &["config", "filter.evil.clean", &filter]);
    f.assert_refused("nested-filter");
    assert!(!marker.exists());
}
#[test]
fn a_clean_nested_repository_is_still_accepted() {
    let f = Fixture::new();
    add_nested_repository(&f);
    let snapshot = CodeSnapshot::create(&f.source, &f.attempt("nested-clean")).unwrap();
    assert!(snapshot.work.exists());
}
/// 没登记成 gitlink 的嵌套仓库：第一次 `add -A` 把它收进私有 index，之后的 `diff`／`add -A`
/// 就会在里面起 git、跑它的过滤器（2026-10-11 在 git 2.53 实测）。闸门按文件系统找嵌套仓库，
/// 不看登记，所以同样在任何 git 子进程之前拒绝。
#[test]
fn an_unregistered_nested_repositorys_filter_is_refused_before_any_git_can_start_it() {
    let f = Fixture::new();
    let nested = f.source.join("embedded");
    init_repository(&nested);
    let marker = f.root.join("UNREGISTERED_FILTER_RAN");
    fs::write(nested.join(".gitattributes"), "* filter=evil\n").unwrap();
    let filter = format!("touch {}; cat", marker.display());
    git(&nested, &["config", "filter.evil.clean", &filter]);
    fs::write(nested.join("file.txt"), "changed\n").unwrap();
    f.assert_refused("unregistered-nested-filter");
    assert!(!marker.exists());
}
#[test]
fn a_clean_unregistered_nested_repository_is_still_accepted() {
    let f = Fixture::new();
    init_repository(&f.source.join("embedded"));
    let snapshot = CodeSnapshot::create(&f.source, &f.attempt("unregistered-clean")).unwrap();
    assert!(snapshot.work.exists());
}
/// 真子模块：`.git` 是指向外层 `.git/modules/<名>` 的小文件，那里的配置带指回本文件夹的
/// `core.worktree`。这是合法布局，必须照常通过。
#[test]
fn a_submodule_layout_is_still_accepted() {
    let f = Fixture::new();
    let upstream = f.root.join("upstream");
    init_repository(&upstream);
    git(
        &f.source,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            upstream.to_str().unwrap(),
            "vendor",
        ],
    );
    git(
        &f.source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "submodule"],
    );
    assert!(f.source.join("vendor/.git").is_file());
    let snapshot = CodeSnapshot::create(&f.source, &f.attempt("submodule")).unwrap();
    assert!(snapshot.work.exists());
}
/// 嵌套仓库的 `.git` 指到外层 `modules/` 之外（这里是另一个仓库）：外层 git 在里面起的子进程会去
/// 动那个仓库，拒绝。
#[test]
fn a_nested_repository_pointing_at_a_foreign_git_directory_is_refused() {
    let f = Fixture::new();
    let foreign = f.root.join("foreign");
    init_repository(&foreign);
    let before = git(&foreign, &["count-objects", "-v"]);
    let nested = f.source.join("vendor");
    fs::create_dir(&nested).unwrap();
    fs::write(
        nested.join(".git"),
        format!("gitdir: {}\n", foreign.join(".git").display()),
    )
    .unwrap();
    f.assert_refused("nested-foreign-pointer");
    assert_eq!(before, git(&foreign, &["count-objects", "-v"]));
}
/// 嵌套仓库的 `core.worktree` 指到别处：嵌套的 git 会去扫一棵没审过的树，拒绝。
#[test]
fn a_nested_core_worktree_pointing_elsewhere_is_refused() {
    let f = Fixture::new();
    let nested = add_nested_repository(&f);
    let elsewhere = f.root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    git(
        &nested,
        &["config", "core.worktree", elsewhere.to_str().unwrap()],
    );
    f.assert_refused("nested-core-worktree");
}
/// 放在检出里的同仓库链接工作树（Claude Code 的 `.claude/worktrees/` 就是这样）：`.git` 指到本仓库
/// 公共目录的 `worktrees/` 下。这是合法布局，必须照常通过。
#[test]
fn a_same_repository_worktree_inside_the_checkout_is_still_accepted() {
    let f = Fixture::new();
    fs::write(f.source.join(".gitignore"), "/wt/\n").unwrap();
    git(&f.source, &["add", ".gitignore"]);
    git(
        &f.source,
        &["-c", "commit.gpgsign=false", "commit", "-qm", "ignore"],
    );
    let inner = f.source.join("wt/inner");
    git(
        &f.source,
        &["worktree", "add", "-q", "--detach", inner.to_str().unwrap()],
    );
    assert!(inner.join(".git").is_file());
    let snapshot = CodeSnapshot::create(&f.source, &f.attempt("inner-worktree")).unwrap();
    assert!(snapshot.work.exists());
}
/// 同仓库链接工作树自己的 `config.worktree` 里藏过滤器：外层 `add -A` 把它收成 gitlink 后，
/// `diff` 在里面起的 git 会读这份配置，所以随嵌套审计一起拒绝。
#[test]
fn a_filter_in_an_inner_worktrees_own_config_is_refused() {
    let f = Fixture::new();
    let inner = f.source.join("wt/inner");
    git(
        &f.source,
        &["worktree", "add", "-q", "--detach", inner.to_str().unwrap()],
    );
    git(&f.source, &["config", "extensions.worktreeConfig", "true"]);
    let marker = f.root.join("INNER_WORKTREE_FILTER_RAN");
    fs::write(inner.join(".gitattributes"), "* filter=evil\n").unwrap();
    let filter = format!("touch {}; cat", marker.display());
    git(
        &inner,
        &["config", "--worktree", "filter.evil.clean", &filter],
    );
    fs::write(inner.join("file.txt"), "changed\n").unwrap();
    f.assert_refused("inner-worktree-filter");
    assert!(!marker.exists());
    // 反证：撤掉过滤器后同一布局照常通过，拒绝来自配置审计而不是布局检查。
    git(
        &inner,
        &["config", "--worktree", "--unset", "filter.evil.clean"],
    );
    let snapshot = CodeSnapshot::create(&f.source, &f.attempt("inner-worktree-clean")).unwrap();
    assert!(snapshot.work.exists());
    assert!(!marker.exists());
}
/// 当前分支的完整引用名、它现在的提交，以及在它之上新做的一个提交：给 `advance_code_ref` 用。
fn branch_step(f: &Fixture) -> (String, String, String) {
    let reference = git(&f.source, &["symbolic-ref", "HEAD"]).trim().to_owned();
    let old = git(&f.source, &["rev-parse", "HEAD"]).trim().to_owned();
    let new = git(
        &f.source,
        &["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "step"],
    )
    .trim()
    .to_owned();
    (reference, old, new)
}
fn assert_advance_refused(f: &Fixture, (reference, old, new): (String, String, String)) {
    assert!(matches!(
        advance_code_ref(&f.source, &reference, &new, &old),
        Err(RunnerError::InvalidExecution)
    ));
}
/// `refs/heads` 链到别的仓库：`update-ref` 会改掉那个仓库的分支、指向它没有的对象
/// （2026-10-11 在 git 2.53 实测），拒绝。
#[test]
fn a_symbolic_link_inside_refs_is_refused_and_the_foreign_repository_is_untouched() {
    let f = Fixture::new();
    let foreign = f.root.join("foreign");
    init_repository(&foreign);
    let before = git(&foreign, &["for-each-ref"]);
    // 先算好推进参数：链上之后源仓库的分支就解析到外来仓库的提交了。
    let step = branch_step(&f);
    fs::remove_dir_all(f.source.join(".git/refs/heads")).unwrap();
    symlink(
        foreign.join(".git/refs/heads"),
        f.source.join(".git/refs/heads"),
    )
    .unwrap();
    f.assert_refused("linked-refs");
    assert_advance_refused(&f, step);
    assert_eq!(before, git(&foreign, &["for-each-ref"]));
}
/// 分支的 reflog 链到仓库外：`update-ref` 会往那个文件追加一行（2026-10-11 在 git 2.53 实测），
/// 拒绝，那个文件不变。
#[test]
fn a_reflog_linked_out_of_the_repository_is_refused_and_the_target_is_untouched() {
    let f = Fixture::new();
    let outside = f.root.join("outside.rc");
    fs::write(&outside, "# untouched\n").unwrap();
    let reference = git(&f.source, &["symbolic-ref", "HEAD"]).trim().to_owned();
    let reflog = f.source.join(".git/logs").join(&reference);
    fs::remove_file(&reflog).unwrap();
    symlink(&outside, &reflog).unwrap();
    f.assert_refused("linked-reflog");
    assert_advance_refused(&f, branch_step(&f));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "# untouched\n");
}
/// reflog 是硬链接：追加的那一行会同时写进别处的同一个文件，拒绝。
#[test]
fn a_hard_linked_reflog_is_refused_and_the_other_name_is_untouched() {
    let f = Fixture::new();
    let outside = f.root.join("outside.rc");
    fs::write(&outside, "# untouched\n").unwrap();
    let reflog = f.source.join(".git/logs/HEAD");
    fs::remove_file(&reflog).unwrap();
    fs::hard_link(&outside, &reflog).unwrap();
    f.assert_refused("hard-linked-reflog");
    assert_advance_refused(&f, branch_step(&f));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "# untouched\n");
}
/// 推进分支写进 reflog 的身份是固定的，检出配置里别人写的 `user.name` 进不去。
#[test]
fn the_reflog_identity_is_fixed_rather_than_taken_from_the_checkout() {
    let f = Fixture::new();
    git(&f.source, &["config", "user.name", "$(touch INJECTED)"]);
    git(&f.source, &["config", "user.email", "evil@invalid"]);
    let (reference, old, new) = branch_step(&f);
    advance_code_ref(&f.source, &reference, &new, &old).unwrap();
    assert_eq!(git(&f.source, &["rev-parse", "HEAD"]).trim(), new);
    let reflog = fs::read_to_string(f.source.join(".git/logs").join(&reference)).unwrap();
    let last = reflog.lines().last().unwrap();
    assert!(last.contains("Studiolo runner <runner@invalid>"), "{last}");
    assert!(!reflog.contains("INJECTED"));
}
/// 本地配置要求提交签名并指定签名程序：快照的 `commit-tree` 不签名，签名程序一次也不执行，
/// 快照照常完成。
#[test]
fn commit_signing_config_never_runs_the_signing_program() {
    let f = Fixture::new();
    let marker = f.root.join("SIGNER_RAN");
    let signer = f.root.join("signer.sh");
    fs::write(
        &signer,
        format!("#!/bin/sh\ntouch {}\nexit 1\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&signer, fs::Permissions::from_mode(0o755)).unwrap();
    git(&f.source, &["config", "commit.gpgSign", "true"]);
    git(
        &f.source,
        &["config", "gpg.program", signer.to_str().unwrap()],
    );
    fs::write(f.source.join("file.txt"), "changed\n").unwrap();
    let snapshot = CodeSnapshot::create(&f.source, &f.attempt("signing")).unwrap();
    assert!(snapshot.work.exists());
    assert!(!marker.exists());
}
