//! Contract checks that require neither an installed CLI nor a privileged Linux backend.
use studiolo_core::{AttemptId, Fence, TaskId};
use studiolo_runner_adapter::{
    AdapterAvailability, BudgetPhase, CliProvider, LinuxIsolation, RealRunner, RunBudget, RunSpec,
    RunnerAdapter, RunnerError, RunnerLaunchConfig,
};

#[test]
fn old_persisted_specs_default_to_stub_only_and_bounded_budgets() {
    let old = serde_json::json!({"task_id":TaskId::new(), "attempt_id":AttemptId::new(),
        "fence":Fence::new(1).unwrap(), "instruction":"demo", "inputs":[]});
    let spec: RunSpec = serde_json::from_value(old).unwrap();
    assert!(spec.execution.is_none());
    assert_eq!(spec.budget, RunBudget::default());
}
#[test]
fn pacing_wrapup_and_hard_ceiling_are_distinct() {
    let budget = RunBudget::default();
    assert!(budget.valid());
    assert_eq!(budget.phase(23, 1), BudgetPhase::Working);
    assert_eq!(budget.phase(24, 1), BudgetPhase::Pacing);
    assert_eq!(budget.phase(120, 1), BudgetPhase::Wrapup);
    assert_eq!(budget.phase(0, 1500), BudgetPhase::Wrapup);
    assert_eq!(budget.phase(160, 1), BudgetPhase::Exhausted);
    assert_eq!(budget.phase(0, 2100), BudgetPhase::Exhausted);
}
#[test]
fn missing_isolation_is_unavailable_for_each_provider_and_cannot_start() {
    for provider in [CliProvider::Claude, CliProvider::Codex] {
        let directory = tempfile::tempdir().unwrap();
        let runner = RealRunner::new(RunnerLaunchConfig {
            provider,
            cli_sha256: "0".repeat(64),
            isolation: LinuxIsolation {
                runtime_root: directory.path().join("missing-root"),
                bubblewrap: "/not-installed/bwrap".into(),
                systemd_run: "/not-installed/systemd-run".into(),
                gateway_socket: directory.path().join("gateway.sock"),
            },
            source_checkout: directory.path().into(),
            attempt_root: directory.path().into(),
            retained_root: directory.path().into(),
            capability_file: directory.path().join("job-capability"),
        });
        assert!(matches!(
            runner.probe().unwrap(),
            AdapterAvailability::Unavailable { .. }
        ));
        let spec = RunSpec {
            task_id: TaskId::new(),
            attempt_id: AttemptId::new(),
            fence: Fence::new(1).unwrap(),
            instruction: "demo".into(),
            inputs: vec![],
            budget: RunBudget::default(),
            execution: None,
        };
        assert!(matches!(
            runner.start(&spec),
            Err(RunnerError::InvalidExecution)
        ));
    }
}
#[test]
fn credential_free_specs_and_errors_have_no_secret_fields() {
    let spec = RunSpec {
        task_id: TaskId::new(),
        attempt_id: AttemptId::new(),
        fence: Fence::new(1).unwrap(),
        instruction: "demo".into(),
        inputs: vec![],
        budget: RunBudget::default(),
        execution: None,
    };
    let value = serde_json::to_value(spec).unwrap();
    assert!(value.get("capability").is_none());
    assert!(value.get("api_key").is_none());
    assert_eq!(
        RunnerError::Unavailable.to_string(),
        "runner isolation is unavailable"
    );
}

#[cfg(unix)]
mod snapshots {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };
    use studiolo_runner_adapter::{CodeSnapshot, SnapshotCleanupRequest, cleanup_orphan_snapshots};

    struct Fixture {
        _temp: tempfile::TempDir,
        source: PathBuf,
        attempts: PathBuf,
        retained: PathBuf,
    }
    fn git(source: &Path, args: &[&str]) -> Vec<u8> {
        let output = Command::new("git")
            .arg("-C")
            .arg(source)
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
        output.stdout
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let source = root.join("source");
            let attempts = root.join("attempts");
            let retained = root.join("retained");
            for path in [&source, &attempts, &retained] {
                fs::create_dir(path).unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            git(&source, &["init", "-q"]);
            fs::write(source.join("file.txt"), "baseline\n").unwrap();
            git(&source, &["add", "."]);
            git(
                &source,
                &["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"],
            );
            Self {
                _temp: temp,
                source,
                attempts,
                retained,
            }
        }
        fn create(&self, name: &str) -> CodeSnapshot {
            let path = self.attempts.join(name);
            fs::create_dir(&path).unwrap();
            CodeSnapshot::create(&self.source, &path).unwrap()
        }
        fn age(&self, name: &str) {
            let path = self.attempts.join(name).join(".studiolo-snapshot.json");
            let mut record: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            record["created_at"] = serde_json::json!(1);
            fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        }
        fn request<'a>(&'a self, active: &'a [PathBuf]) -> SnapshotCleanupRequest<'a> {
            SnapshotCleanupRequest {
                source_checkout: &self.source,
                attempt_root: &self.attempts,
                retained_root: &self.retained,
                active_attempts: active,
                now_unix_seconds: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
                minimum_age_seconds: 300,
            }
        }
    }

    #[test]
    fn full_dirty_tree_is_shared_but_attempt_writes_index_and_branches_are_isolated() {
        let f = Fixture::new();
        fs::write(f.source.join("file.txt"), "dirty baseline\n").unwrap();
        fs::write(f.source.join("untracked.txt"), "untracked baseline\n").unwrap();
        let source_index = fs::read(f.source.join(".git/index")).unwrap();
        let source_head = git(&f.source, &["rev-parse", "HEAD"]);
        let first = f.create("first");
        let cached = fs::read(f.attempts.join(".studiolo-snapshot-cache.json")).unwrap();
        let second = f.create("second");
        assert_eq!(first.base, second.base);
        assert_eq!(
            cached,
            fs::read(f.attempts.join(".studiolo-snapshot-cache.json")).unwrap()
        );
        assert_eq!(
            fs::read(second.work.join("untracked.txt")).unwrap(),
            b"untracked baseline\n"
        );
        fs::write(first.work.join("file.txt"), "first candidate\n").unwrap();
        assert_eq!(
            fs::read(second.work.join("file.txt")).unwrap(),
            b"dirty baseline\n"
        );
        assert_eq!(source_index, fs::read(f.source.join(".git/index")).unwrap());
        assert_eq!(source_head, git(&f.source, &["rev-parse", "HEAD"]));
        fs::write(f.source.join("untracked.txt"), "new input bytes\n").unwrap();
        let third = f.create("third");
        assert_ne!(first.base, third.base);
        CodeSnapshot::invalidate_cache(&f.attempts).unwrap();
        assert!(!f.attempts.join(".studiolo-snapshot-cache.json").exists());
        assert!(first.work.exists() && second.work.exists());
    }
    #[test]
    fn cache_metadata_cannot_substitute_an_unrelated_commit() {
        let f = Fixture::new();
        f.create("first");
        let path = f.attempts.join(".studiolo-snapshot-cache.json");
        let mut cache: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        cache["commit"] = serde_json::json!("0".repeat(40));
        fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
        let next = f.attempts.join("second");
        fs::create_dir(&next).unwrap();
        assert!(CodeSnapshot::create(&f.source, &next).is_err());
        assert!(!next.join("work").exists());
    }
    #[test]
    fn orphan_cleanup_retains_patch_before_removing_only_old_inactive_registered_copy() {
        let f = Fixture::new();
        let orphan = f.create("orphan");
        let active = f.create("active");
        let young = f.create("young");
        fs::write(orphan.work.join("file.txt"), "orphan candidate\n").unwrap();
        f.age("orphan");
        f.age("active");
        let active_paths = [f.attempts.join("active")];
        let report = cleanup_orphan_snapshots(&f.request(&active_paths)).unwrap();
        assert_eq!(report.removed, [f.attempts.join("orphan")]);
        assert!(report.blocked.is_empty());
        assert_eq!(report.protected.len(), 2);
        assert!(!orphan.work.exists());
        assert!(active.work.exists() && young.work.exists());
        let evidence = &report.retained[0];
        let patch = fs::read(&evidence.retained_path).unwrap();
        assert!(String::from_utf8_lossy(&patch).contains("+orphan candidate"));
        let digest: String = Sha256::digest(&patch)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, evidence.sha256);
        let durable_metadata: studiolo_runner_adapter::PatchMetadata = serde_json::from_slice(
            &fs::read(
                evidence
                    .retained_path
                    .with_file_name("orphan-orphan-unlanded.patch.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(&durable_metadata, evidence);
        let worktrees =
            String::from_utf8(git(&f.source, &["worktree", "list", "--porcelain"])).unwrap();
        assert!(!worktrees.contains(orphan.work.to_str().unwrap()));
        assert!(
            cleanup_orphan_snapshots(&f.request(&active_paths))
                .unwrap()
                .removed
                .is_empty()
        );
    }
    #[test]
    fn unknown_replaced_or_symlinked_directories_are_never_deleted() {
        let f = Fixture::new();
        let registered = f.create("replaced");
        f.age("replaced");
        let record = fs::read(f.attempts.join("replaced/.studiolo-snapshot.json")).unwrap();
        fs::rename(
            f.attempts.join("replaced"),
            f.attempts.join("original-moved"),
        )
        .unwrap();
        fs::create_dir(f.attempts.join("replaced")).unwrap();
        fs::write(f.attempts.join("replaced/.studiolo-snapshot.json"), record).unwrap();
        fs::set_permissions(
            f.attempts.join("replaced/.studiolo-snapshot.json"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        fs::create_dir(f.attempts.join("unknown")).unwrap();
        symlink(&f.source, f.attempts.join("linked")).unwrap();
        let report = cleanup_orphan_snapshots(&f.request(&[])).unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(report.blocked.len(), 4);
        assert!(f.source.join("file.txt").exists());
        assert!(f.attempts.join("original-moved/work").exists());
        assert!(registered.work.parent().unwrap().exists());
    }
    #[test]
    fn unsafe_work_or_failed_patch_preservation_keeps_the_original_tree() {
        let f = Fixture::new();
        let snapshot = f.create("unsafe");
        f.age("unsafe");
        symlink(f.source.join("file.txt"), snapshot.work.join("link.txt")).unwrap();
        let report = cleanup_orphan_snapshots(&f.request(&[])).unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(report.blocked, [f.attempts.join("unsafe")]);
        assert!(snapshot.work.exists());
        fs::remove_file(snapshot.work.join("link.txt")).unwrap();
        fs::hard_link(
            f.source.join("file.txt"),
            snapshot.work.join("hardlink.txt"),
        )
        .unwrap();
        let report = cleanup_orphan_snapshots(&f.request(&[])).unwrap();
        assert!(report.removed.is_empty());
        assert!(snapshot.work.exists());
        fs::remove_file(snapshot.work.join("hardlink.txt")).unwrap();
        fs::write(
            f.retained.join("unsafe-orphan-unlanded.patch"),
            "different existing evidence",
        )
        .unwrap();
        fs::set_permissions(
            f.retained.join("unsafe-orphan-unlanded.patch"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let report = cleanup_orphan_snapshots(&f.request(&[])).unwrap();
        assert!(report.removed.is_empty());
        assert!(snapshot.work.exists());
        assert_eq!(
            fs::read(f.retained.join("unsafe-orphan-unlanded.patch")).unwrap(),
            b"different existing evidence"
        );
    }
    #[test]
    fn conflict_marked_orphans_remain_available_and_their_patch_is_preserved() {
        let f = Fixture::new();
        let snapshot = f.create("conflict");
        f.age("conflict");
        fs::write(
            snapshot.work.join("file.txt"),
            "<<<<<<< left\nleft\n=======\nright\n>>>>>>> right\n",
        )
        .unwrap();
        let report = cleanup_orphan_snapshots(&f.request(&[])).unwrap();
        assert_eq!(report.blocked, [f.attempts.join("conflict")]);
        assert!(report.removed.is_empty());
        assert!(snapshot.work.exists());
        assert!(f.retained.join("conflict-orphan-unlanded.patch").exists());
    }
    #[test]
    fn cleanup_rejects_overlapping_roots_invalid_active_paths_and_unsafe_minimum_age() {
        let f = Fixture::new();
        let snapshot = f.create("keep");
        f.age("keep");
        let mut request = f.request(&[]);
        request.retained_root = &f.attempts;
        assert!(cleanup_orphan_snapshots(&request).is_err());
        let invalid = [f.source.clone()];
        let request = f.request(&invalid);
        assert!(cleanup_orphan_snapshots(&request).is_err());
        let mut request = f.request(&[]);
        request.minimum_age_seconds = 0;
        assert!(cleanup_orphan_snapshots(&request).is_err());
        assert!(snapshot.work.exists());
    }
}
