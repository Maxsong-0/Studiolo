//! 仓库副作用只在私有临时夹具中演练，两种 `CLI` 都不调用。
use super::{hash, landing_store::Lease};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use studiolo_runner_adapter::{CodeLandingPlan, apply_code_patch};

fn git(repo: &Path, args: &[&str]) -> Vec<u8> {
    let result = Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Landing test",
            "-c",
            "user.email=landing@invalid",
            "-C",
        ])
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(result.status.success());
    result.stdout
}
fn oid(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args))
        .unwrap()
        .trim()
        .to_owned()
}
fn fixture() -> (tempfile::TempDir, CodeLandingPlan) {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    fs::write(repo.join("a.txt"), "head\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-qm", "initial"]);
    let head = oid(&repo, &["rev-parse", "HEAD"]);
    fs::write(repo.join("a.txt"), "dirty baseline\n").unwrap();
    fs::write(repo.join("untracked.txt"), "approved input\n").unwrap();
    git(&repo, &["add", "-A"]);
    let before = oid(&repo, &["write-tree"]);
    fs::write(repo.join("a.txt"), "candidate\n").unwrap();
    git(&repo, &["add", "-A"]);
    let after = oid(&repo, &["write-tree"]);
    let patch = git(&repo, &["diff", "--binary", &before, &after]);
    let patch_path = root.path().join("retained.patch");
    fs::write(&patch_path, &patch).unwrap();
    git(&repo, &["reset", "--mixed", "HEAD"]);
    fs::write(repo.join("a.txt"), "dirty baseline\n").unwrap();
    let plan = CodeLandingPlan {
        repository: repo,
        expected_head: head,
        expected_tree: before,
        expected_after_tree: after,
        patch_sha256: hash(&patch),
        patch_path,
    };
    (root, plan)
}
#[test]
fn dirty_and_untracked_baseline_cas_preserves_index_and_refs() {
    let (_root, plan) = fixture();
    let index = fs::read(plan.repository.join(".git/index")).unwrap();
    assert!(apply_code_patch(&plan).unwrap().applied);
    assert_eq!(fs::read(plan.repository.join(".git/index")).unwrap(), index);
    assert_eq!(
        oid(&plan.repository, &["rev-parse", "HEAD"]),
        plan.expected_head
    );
    assert_eq!(
        fs::read_to_string(plan.repository.join("a.txt")).unwrap(),
        "candidate\n"
    );
    assert!(plan.verify_landed().is_ok());
    assert!(plan.patch_path.is_file());
}
#[test]
fn new_untracked_input_or_stale_head_blocks_patch_before_effects() {
    let (_root, mut plan) = fixture();
    fs::write(plan.repository.join("late.txt"), "human edit\n").unwrap();
    assert!(apply_code_patch(&plan).is_err());
    assert_eq!(
        fs::read_to_string(plan.repository.join("a.txt")).unwrap(),
        "dirty baseline\n"
    );
    fs::remove_file(plan.repository.join("late.txt")).unwrap();
    plan.expected_head = "0".repeat(40);
    assert!(apply_code_patch(&plan).is_err());
    assert!(plan.patch_path.is_file());
}
#[test]
fn repository_lease_excludes_snapshot_and_landing_and_keeps_immutable_intent() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let repo = root.path().join("repo");
    fs::create_dir(&state).unwrap();
    fs::create_dir(&repo).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let state = fs::canonicalize(state).unwrap();
    let repo = fs::canonicalize(repo).unwrap();
    let lease = Lease::acquire(&state, &repo).unwrap();
    assert!(Lease::acquire(&state, &repo).is_err());
    lease.write("attempt", "intent", &"first").unwrap();
    assert!(lease.write("attempt", "intent", &"changed").is_err());
    assert_eq!(
        lease
            .read::<String>("attempt", "intent")
            .unwrap()
            .as_deref(),
        Some("first")
    );
    let inherited = lease.duplicate_descriptor_for_test().unwrap();
    drop(lease);
    let next = Lease::acquire(&state, &repo).unwrap();
    drop(inherited);
    assert!(Lease::acquire(&state, &repo).is_err());
    drop(next);
    assert!(Lease::acquire(&state, &repo).is_ok());
}

#[test]
fn an_explicit_noop_candidate_still_requires_the_exact_dirty_baseline() {
    let (_root, mut plan) = fixture();
    plan.expected_after_tree = plan.expected_tree.clone();
    fs::write(&plan.patch_path, []).unwrap();
    plan.patch_sha256 = hash(&[]);
    assert!(apply_code_patch(&plan).unwrap().applied);
    fs::write(plan.repository.join("untracked.txt"), "later change\n").unwrap();
    assert!(apply_code_patch(&plan).is_err());
}
