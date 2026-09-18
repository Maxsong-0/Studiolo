//! 真实 IO 契约：只在私有 0700 目录里操作固定文件名，原子替换、失败关闭。

use super::support::manifest;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{PermissionsExt as _, symlink};
use std::path::PathBuf;
use studiolo_backup::store::model::{BackupStateStore, PersistedState, StoreError};
use studiolo_backup::store::storage::{
    BackupStore, MANIFEST_FILE, MANIFEST_TEMP_FILE, STATE_FILE, STATE_MAX_BYTES, STATE_TEMP_FILE,
};
use studiolo_core::VaultUuid;
use tempfile::TempDir;

struct Private {
    _temp: TempDir,
    root: PathBuf,
}

impl Private {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t048-backup-store-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).expect("canonical temp"))
            .expect("tempdir");
        let root = temp.path().join("state");
        fs::create_dir(&root).expect("create root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("chmod");
        Self { _temp: temp, root }
    }

    fn open(&self, initialize: bool) -> Result<BackupStore, StoreError> {
        BackupStore::open(&self.root, initialize)
    }
}

fn overwrite(path: &std::path::Path, bytes: &[u8]) {
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .expect("open fixed file");
    file.write_all(bytes).expect("overwrite");
}

#[test]
fn initializing_creates_the_fixed_state_file_and_reopening_finds_it() {
    let private = Private::new();
    let store = private.open(true).expect("initialize");

    let state = store.load_state().expect("load").expect("state");
    assert_eq!(state, PersistedState::initial());
    assert!(private.root.join(STATE_FILE).is_file());
    assert!(!private.root.join(STATE_TEMP_FILE).exists());
    assert!(!private.root.join(MANIFEST_FILE).exists());

    let reopened = private.open(false).expect("reopen");
    assert_eq!(reopened.load_state().expect("load"), Some(state));
}

#[test]
fn opening_an_empty_directory_without_initialize_reports_a_missing_store() {
    let private = Private::new();
    assert_eq!(private.open(false).err(), Some(StoreError::MissingStore));
}

#[test]
fn initializing_an_existing_store_is_refused_instead_of_overwriting_it() {
    let private = Private::new();
    private.open(true).expect("initialize");
    assert_eq!(private.open(true).err(), Some(StoreError::AlreadyExists));
}

#[test]
fn a_world_readable_directory_is_not_a_private_store() {
    let private = Private::new();
    private.open(true).expect("initialize");
    fs::set_permissions(&private.root, fs::Permissions::from_mode(0o755)).expect("chmod");

    assert_eq!(private.open(false).err(), Some(StoreError::UnsafeStorage));
}

#[test]
fn the_state_file_is_created_with_owner_only_permissions() {
    let private = Private::new();
    private.open(true).expect("initialize");

    let mode = fs::metadata(private.root.join(STATE_FILE))
        .expect("stat")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o600);
}

#[test]
fn saving_replaces_the_state_atomically_and_leaves_no_temporary_behind() {
    let private = Private::new();
    let store = private.open(true).expect("initialize");
    let mut state = PersistedState::initial();
    state.next_sequence = 9;
    state.seed_verified = true;

    store.save_state(&state).expect("save");

    assert!(!private.root.join(STATE_TEMP_FILE).exists());
    assert_eq!(
        private
            .open(false)
            .expect("reopen")
            .load_state()
            .expect("load"),
        Some(state)
    );
}

#[test]
fn a_leftover_temporary_from_a_crashed_write_is_replaced_rather_than_blocking_the_store() {
    let private = Private::new();
    let store = private.open(true).expect("initialize");
    fs::write(private.root.join(STATE_TEMP_FILE), b"half written").expect("stale temp");
    fs::set_permissions(
        private.root.join(STATE_TEMP_FILE),
        fs::Permissions::from_mode(0o600),
    )
    .expect("chmod");

    let mut state = PersistedState::initial();
    state.next_sequence = 3;
    store.save_state(&state).expect("save over the stale temp");

    assert!(!private.root.join(STATE_TEMP_FILE).exists());
    assert_eq!(store.load_state().expect("load"), Some(state));
}

#[test]
fn a_half_written_state_is_reported_as_corrupt_not_as_an_empty_store() {
    let private = Private::new();
    private.open(true).expect("initialize");
    overwrite(&private.root.join(STATE_FILE), b"{\"schema_version\":1,");

    let store = private.open(false).expect("reopen");
    assert_eq!(store.load_state().err(), Some(StoreError::CorruptStore));
}

#[test]
fn an_unknown_field_or_schema_version_fails_closed() {
    let private = Private::new();
    private.open(true).expect("initialize");
    overwrite(
        &private.root.join(STATE_FILE),
        br#"{"schema_version":1,"next_sequence":1,"jobs":[],"last_scheduled":null,
            "last_daily_immutable":null,"seed_verified":false,"surprise":true}"#,
    );
    assert_eq!(
        private.open(false).expect("reopen").load_state().err(),
        Some(StoreError::CorruptStore)
    );

    overwrite(
        &private.root.join(STATE_FILE),
        br#"{"schema_version":99,"next_sequence":1,"jobs":[],"last_scheduled":null,
            "last_daily_immutable":null,"seed_verified":false}"#,
    );
    assert_eq!(
        private.open(false).expect("reopen").load_state().err(),
        Some(StoreError::UnsupportedSchema)
    );
}

#[test]
fn a_state_whose_job_sequence_runs_past_the_allocator_is_corrupt() {
    let private = Private::new();
    private.open(true).expect("initialize");
    overwrite(
        &private.root.join(STATE_FILE),
        br#"{"schema_version":1,"next_sequence":1,
            "jobs":[{"key":{"sequence":5,"checkpoint_id":"cp-1"},"state":{"state":"queued"},
            "checkpoint_at":null,"remote_verified_at":null,"finalized_at":null}],
            "last_scheduled":null,"last_daily_immutable":null,"seed_verified":false}"#,
    );

    assert_eq!(
        private.open(false).expect("reopen").load_state().err(),
        Some(StoreError::CorruptStore)
    );
}

#[test]
fn a_state_larger_than_the_fixed_limit_is_refused_before_it_is_written() {
    let private = Private::new();
    let store = private.open(true).expect("initialize");
    let mut state = PersistedState::initial();
    // 一个远超上限的 jobs 列表；不靠磁盘去发现问题。
    let template = serde_json::to_vec(&state).expect("encode");
    assert!((template.len() as u64) < STATE_MAX_BYTES);
    state.next_sequence = u64::MAX;
    for sequence in 1..40_000_u64 {
        state.jobs.push(job(sequence));
    }

    assert_eq!(
        store.save_state(&state).err(),
        Some(StoreError::LimitExceeded)
    );
    assert_eq!(
        store.load_state().expect("load"),
        Some(PersistedState::initial()),
        "被拒绝的写入不改变既有内容"
    );
}

fn job(sequence: u64) -> studiolo_backup::worker::state::JobRecord {
    studiolo_backup::worker::state::JobRecord::queued(studiolo_backup::worker::state::JobKey {
        sequence: std::num::NonZeroU64::new(sequence).expect("sequence"),
        checkpoint_id: studiolo_core::checkpoint::CheckpointId::parse(format!("cp-{sequence:08}"))
            .expect("checkpoint id"),
    })
}

#[test]
fn committing_the_final_manifest_writes_the_fixed_file_and_survives_a_reopen() {
    let private = Private::new();
    let store = private.open(true).expect("initialize");
    let temporary = TempDir::new().expect("tempdir");
    let manifest = manifest(VaultUuid::new(), VaultUuid::new(), temporary.path());

    assert_eq!(store.load_final_manifest().expect("load"), None);
    store.commit_final_manifest(&manifest).expect("commit");

    assert!(private.root.join(MANIFEST_FILE).is_file());
    assert!(!private.root.join(MANIFEST_TEMP_FILE).exists());
    assert_eq!(
        private
            .open(false)
            .expect("reopen")
            .load_final_manifest()
            .expect("load"),
        Some(manifest)
    );
}

#[test]
fn an_invalid_manifest_never_reaches_the_fixed_file() {
    let private = Private::new();
    let store = private.open(true).expect("initialize");
    let temporary = TempDir::new().expect("tempdir");
    let mut manifest = manifest(VaultUuid::new(), VaultUuid::new(), temporary.path());
    manifest.schema_version = 42;

    assert_eq!(
        store.commit_final_manifest(&manifest).err(),
        Some(StoreError::CorruptStore)
    );
    assert!(!private.root.join(MANIFEST_FILE).exists());
}

#[test]
fn a_symlink_standing_in_for_a_fixed_file_is_refused() {
    let private = Private::new();
    private.open(true).expect("initialize");
    let elsewhere = private.root.join("elsewhere.json");
    fs::write(&elsewhere, b"{}").expect("write");
    fs::remove_file(private.root.join(MANIFEST_FILE)).ok();
    symlink(&elsewhere, private.root.join(MANIFEST_FILE)).expect("symlink");

    let store = private.open(false).expect("reopen");
    assert_eq!(
        store.load_final_manifest().err(),
        Some(StoreError::UnsafeStorage)
    );
}

#[test]
fn a_relative_root_path_is_refused_without_touching_anything() {
    assert_eq!(
        BackupStore::open(std::path::Path::new("relative/state"), true).err(),
        Some(StoreError::UnsafeStorage)
    );
}
