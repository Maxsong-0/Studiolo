//! 真实SQLite与跨进程契约；全部在108执行，不以模拟锁代替写屏障。
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use studiolo_policy::{
    DesiredInput, PolicyAgentStore, PolicyCheckpointHandle, PolicyError, PolicyKind, PolicyStore,
    ProposalInput, Submitter,
};

const NOW: &str = "2026-11-13T12:00:00Z";
fn private(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}
fn fixture() -> (tempfile::TempDir, PathBuf, PolicyStore) {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let directory = private(temp.path(), "source");
    let store = PolicyStore::open(&directory).unwrap();
    (temp, directory, store)
}
fn until(millis: u64) -> Instant {
    Instant::now() + Duration::from_millis(millis)
}
fn submit(store: &PolicyStore, body: &str) -> String {
    store
        .submit_desired(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "CLAUDE.md".into(),
                body: Some(body.into()),
            },
            Submitter::Owner,
            NOW,
        )
        .unwrap()
        .id
}
fn ids(store: &PolicyStore) -> Vec<String> {
    store
        .desired_list(None, 500)
        .unwrap()
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[test]
fn export_preserves_full_content_effective_and_desired_order_with_sparse_rowids() {
    let (temp, directory, store) = fixture();
    store
        .bootstrap(
            DesiredInput {
                kind: PolicyKind::Rules,
                subject: "CLAUDE.md".into(),
                body: Some("原规则".into()),
            },
            NOW,
        )
        .unwrap();
    let first = submit(&store, "先到");
    let second = submit(&store, "后到");
    let raw = rusqlite::Connection::open(directory.join("policy.sqlite3")).unwrap();
    raw.execute("UPDATE policy_desired SET rowid=rowid+1000", [])
        .unwrap();
    let handle = store.checkpoint_handle().unwrap();
    handle.freeze(until(3000)).unwrap();
    let target = private(temp.path(), "export");
    let receipt = handle.export(&target, until(3000)).unwrap();
    assert!(receipt.is_verified());
    let copy = PolicyStore::open_existing(&target).unwrap();
    assert_eq!(ids(&copy), ids(&store));
    assert_eq!(copy.effective().unwrap(), store.effective().unwrap());
    assert_eq!(
        copy.desired(&first).unwrap(),
        store.desired(&first).unwrap()
    );
    handle.thaw().unwrap();
    handle.thaw().unwrap();
    let hash = copy.effective().unwrap().effective_hash;
    assert!(
        matches!(copy.activate(&first, &hash, "owner", NOW), Err(PolicyError::NewerDesiredFromSameSubmitter { newer }) if newer == vec![second])
    );
    let mode: String = rusqlite::Connection::open(target.join("policy.sqlite3"))
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
}

#[test]
fn readers_and_existing_store_startup_remain_available_during_freeze() {
    let (_temp, directory, store) = fixture();
    let handle = store.checkpoint_handle().unwrap();
    handle.freeze(until(1000)).unwrap();
    let start = Instant::now();
    PolicyAgentStore::open_existing(&directory)
        .unwrap()
        .effective()
        .unwrap();
    PolicyStore::open_existing(&directory)
        .unwrap()
        .effective()
        .unwrap();
    assert!(start.elapsed() < Duration::from_millis(1500));
    handle.thaw().unwrap();
}

#[test]
fn old_reader_blocks_wal_proof_and_expiry_does_not_create_a_target_file() {
    let (temp, directory, store) = fixture();
    let reader = rusqlite::Connection::open(directory.join("policy.sqlite3")).unwrap();
    reader
        .execute_batch("BEGIN; SELECT count(*) FROM policy_desired;")
        .unwrap();
    submit(&store, "读者之后的变更");
    let handle = store.checkpoint_handle().unwrap();
    handle.freeze(until(1000)).unwrap();
    let target = private(temp.path(), "copy");
    let start = Instant::now();
    assert_eq!(
        handle.export(&target, until(120)).unwrap_err(),
        PolicyError::Busy
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    reader.execute_batch("ROLLBACK").unwrap();
    assert!(handle.export(&target, until(2000)).unwrap().is_verified());
    handle.thaw().unwrap();
}

#[test]
fn target_must_be_new_empty_private_and_source_binding_must_stay_fixed() {
    let (temp, directory, store) = fixture();
    let handle = store.checkpoint_handle().unwrap();
    handle.freeze(until(1000)).unwrap();
    let nonempty = private(temp.path(), "nonempty");
    fs::write(nonempty.join("unrelated"), "保留").unwrap();
    assert_eq!(
        handle.export(&nonempty, until(1000)).unwrap_err(),
        PolicyError::CheckpointTarget
    );
    let public = private(temp.path(), "public");
    fs::set_permissions(&public, fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(
        handle.export(&public, until(1000)).unwrap_err(),
        PolicyError::UnsafeStorage
    );
    let exists = private(temp.path(), "existing");
    let existing = PolicyStore::open(&exists).unwrap();
    submit(&existing, "不得覆盖");
    let expected = ids(&existing);
    assert_eq!(
        handle.export(&exists, until(1000)).unwrap_err(),
        PolicyError::CheckpointTarget
    );
    assert_eq!(ids(&existing), expected);
    let copy = private(temp.path(), "copy");
    fs::rename(directory.join("policy.sqlite3"), directory.join("old")).unwrap();
    fs::copy(
        exists.join("policy.sqlite3"),
        directory.join("policy.sqlite3"),
    )
    .unwrap();
    fs::set_permissions(
        directory.join("policy.sqlite3"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert_eq!(
        handle.export(&copy, until(1000)).unwrap_err(),
        PolicyError::UnsafeStorage
    );
    handle.thaw().unwrap();
}

struct Process {
    child: Child,
    output: BufReader<std::process::ChildStdout>,
}
impl Process {
    fn start(directory: &Path, mode: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "checkpoint_child", "--nocapture"])
            .env("STUDIOLO_POLICY_CHECKPOINT_CHILD", directory)
            .env("STUDIOLO_POLICY_CHECKPOINT_MODE", mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut process = Self { child, output };
        process.wait_for("POLICY_READY");
        process
    }
    fn wait_for(&mut self, marker: &str) {
        loop {
            let mut line = String::new();
            assert_ne!(
                self.output.read_line(&mut line).unwrap(),
                0,
                "子进程未返回预期回执"
            );
            if line.contains(marker) {
                break;
            }
        }
    }
    fn command(&mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"go\n")
            .unwrap();
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn another_process_waits_then_gets_busy_without_landing_and_can_retry_after_thaw() {
    let (temp, directory, store) = fixture();
    let mut process = Process::start(&directory, "propose");
    let handle = store.checkpoint_handle().unwrap();
    handle.freeze(until(2000)).unwrap();
    process.command();
    process.wait_for("POLICY_BUSY");
    assert!(ids(&store).is_empty());
    let target = private(temp.path(), "copy");
    assert!(handle.export(&target, until(2000)).unwrap().is_verified());
    assert!(ids(&PolicyStore::open_existing(&target).unwrap()).is_empty());
    handle.thaw().unwrap();
    process.command();
    process.wait_for("POLICY_LANDED");
    assert_eq!(ids(&store).len(), 1);
}

#[test]
fn a_long_writer_times_out_within_budget_and_a_crash_releases_its_lock() {
    let (_temp, directory, store) = fixture();
    let mut process = Process::start(&directory, "reserve");
    let handle = store.checkpoint_handle().unwrap();
    let start = Instant::now();
    assert_eq!(handle.freeze(until(500)), Err(PolicyError::Busy));
    assert!(start.elapsed() >= Duration::from_millis(500));
    assert!(start.elapsed() < Duration::from_millis(1400));
    process.child.kill().unwrap();
    process.child.wait().unwrap();
    handle.freeze(until(1000)).unwrap();
    handle.thaw().unwrap();
    submit(&store, "崩溃后可写");
}

#[test]
fn no_frozen_window_or_expired_deadline_cannot_export_and_dropping_unfreezes() {
    let (temp, directory, store) = fixture();
    let handle = PolicyCheckpointHandle::open_existing(&directory).unwrap();
    let target = private(temp.path(), "copy");
    assert_eq!(
        handle.export(&target, until(1000)).unwrap_err(),
        PolicyError::StoreUnavailable
    );
    assert_eq!(handle.freeze(Instant::now()), Err(PolicyError::Busy));
    handle.freeze(until(1000)).unwrap();
    assert_eq!(handle.freeze(until(100)), Err(PolicyError::Busy));
    drop(handle);
    submit(&store, "句柄释放后可写");
}

#[test]
fn checkpoint_child() {
    let Some(directory) = std::env::var_os("STUDIOLO_POLICY_CHECKPOINT_CHILD") else {
        return;
    };
    if std::env::var("STUDIOLO_POLICY_CHECKPOINT_MODE").unwrap() == "reserve" {
        let connection =
            rusqlite::Connection::open(PathBuf::from(directory).join("policy.sqlite3")).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        println!("POLICY_READY");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        return;
    }
    let store = PolicyAgentStore::open_existing(Path::new(&directory)).unwrap();
    println!("POLICY_READY");
    std::io::stdout().flush().unwrap();
    for expected_busy in [true, false] {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        let start = Instant::now();
        let result = store.propose(
            "claude",
            "01INSTANCE",
            ProposalInput {
                kind: PolicyKind::Rules,
                subject: "CLAUDE.md".into(),
                body: "跨进程提案".into(),
            },
            NOW,
        );
        if expected_busy {
            assert_eq!(result.unwrap_err(), PolicyError::Busy);
            assert!(start.elapsed() >= Duration::from_secs(4));
            println!("POLICY_BUSY");
        } else {
            result.unwrap();
            println!("POLICY_LANDED");
        }
        std::io::stdout().flush().unwrap();
    }
}
