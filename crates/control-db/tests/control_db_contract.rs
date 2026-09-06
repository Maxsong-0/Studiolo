//! Contract tests for the SQLite control plane.

use serde_json::json;
use studiolo_control_db::{
    AttemptPhase, ControlDb, DbError, SnapshotEntryRecord, SnapshotRecord, TaskStatus,
    TerminalReservation,
};
use studiolo_core::{Fence, TaskId, VaultUuid};
use tempfile::TempDir;

fn database_path(temp: &TempDir) -> std::path::PathBuf {
    temp.path().join("control.sqlite3")
}

fn snapshot(id: &str, vault_uuid: VaultUuid) -> SnapshotRecord {
    SnapshotRecord {
        snapshot_id: id.to_owned(),
        vault_uuid,
        entries: vec![SnapshotEntryRecord {
            path: "notes/input.md".to_owned(),
            revision: "abc123".to_owned(),
            effective_label_json: json!({"classification": "internal"}),
        }],
    }
}

#[test]
fn empty_database_migrates_and_enforces_runtime_pragmas() {
    let temp = TempDir::new().expect("tempdir");
    let vault_uuid = VaultUuid::new();
    let db = ControlDb::open(database_path(&temp), vault_uuid).expect("open database");

    assert_eq!(db.schema_version().expect("schema version"), 2);
    assert_eq!(db.bound_vault_uuid().expect("vault binding"), vault_uuid);
    assert_eq!(db.pragma("journal_mode").expect("journal mode"), "wal");
    assert_eq!(db.pragma("synchronous").expect("sync mode"), "2");
    assert_eq!(db.pragma("foreign_keys").expect("foreign keys"), "1");
}

#[test]
fn database_refuses_a_different_vault_identity() {
    let temp = TempDir::new().expect("tempdir");
    let path = database_path(&temp);
    ControlDb::open(&path, VaultUuid::new()).expect("initial binding");

    assert!(matches!(
        ControlDb::open(&path, VaultUuid::new()),
        Err(DbError::VaultBindingMismatch { .. })
    ));
}

#[test]
fn existing_v1_database_upgrades_without_losing_runtime_records() {
    let temp = TempDir::new().unwrap();
    let path = database_path(&temp);
    let task = TaskId::new();
    let vault = VaultUuid::new();
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY, applied_at_ms INTEGER NOT NULL);
             INSERT INTO schema_migrations VALUES(1, 0);
             CREATE TABLE tasks(task_id TEXT PRIMARY KEY, definition_json TEXT NOT NULL, status TEXT NOT NULL,
                 active_attempt TEXT, last_attempt TEXT,
                 created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE vault_binding(singleton INTEGER PRIMARY KEY CHECK(singleton = 1), vault_uuid TEXT NOT NULL);",
        ).unwrap();
        connection.execute("INSERT INTO tasks(task_id,definition_json,status,created_at_ms,updated_at_ms) VALUES(?1,?2,'draft',42,42)",
            rusqlite::params![task.to_string(), "{\"title\":\"v1 task\"}"]).unwrap();
        connection
            .execute(
                "INSERT INTO vault_binding VALUES(1,?1)",
                [vault.to_string()],
            )
            .unwrap();
    }
    let db = ControlDb::open(&path, vault).unwrap();
    assert_eq!(db.schema_version().unwrap(), 2);
    let record = db.task(task).unwrap().unwrap();
    assert_eq!(record.definition_json["title"], "v1 task");
    assert_eq!(record.created_at_ms, 42);
    assert!(db.pending_terminal_audits().unwrap().is_empty());
}

#[test]
fn claim_is_transactional_unique_and_fences_are_monotonic() {
    let temp = TempDir::new().expect("tempdir");
    let vault_uuid = VaultUuid::new();
    let mut db = ControlDb::open(database_path(&temp), vault_uuid).expect("open database");
    let first_task = TaskId::new();
    let second_task = TaskId::new();
    db.create_task(first_task, &json!({"title": "first"}), 10)
        .expect("create first");
    db.create_task(second_task, &json!({"title": "second"}), 11)
        .expect("create second");
    db.queue_task(first_task, 12).expect("queue first");
    db.queue_task(second_task, 13).expect("queue second");

    let first = db
        .claim(
            first_task,
            "worker-a",
            snapshot("snapshot-a", vault_uuid),
            20,
            120,
        )
        .expect("claim first");
    assert_eq!(first.fence, Fence::new(1).unwrap());
    assert_eq!(first.phase, AttemptPhase::Running);
    assert_eq!(
        db.task(first_task)
            .unwrap()
            .unwrap()
            .input_snapshot
            .as_deref(),
        Some("snapshot-a")
    );
    assert!(
        db.claim(
            first_task,
            "worker-b",
            snapshot("snapshot-b", vault_uuid),
            21,
            121
        )
        .is_err()
    );

    db.cancel_task(first_task, 30).expect("cancel first");
    assert!(
        !db.is_attempt_active(first_task, first.attempt_id, first.fence)
            .expect("guard query")
    );

    let second = db
        .claim(
            second_task,
            "worker-b",
            snapshot("snapshot-c", vault_uuid),
            31,
            131,
        )
        .expect("claim second");
    assert!(second.fence > first.fence);
}

#[test]
fn terminal_flow_is_two_phase_idempotent_and_emits_one_projection() {
    let temp = TempDir::new().expect("tempdir");
    let vault_uuid = VaultUuid::new();
    let mut db = ControlDb::open(database_path(&temp), vault_uuid).expect("open database");
    let task_id = TaskId::new();
    db.create_task(task_id, &json!({"title": "task"}), 1)
        .expect("create");
    db.queue_task(task_id, 2).expect("queue");
    let attempt = db
        .claim(task_id, "worker", snapshot("snapshot", vault_uuid), 3, 103)
        .expect("claim");
    let reservation = TerminalReservation {
        report_json: json!({"outcome": "needs_review"}),
        artifacts_json: json!([{"name": "result.md"}]),
    };

    db.reserve_terminal(task_id, attempt.attempt_id, attempt.fence, &reservation, 4)
        .expect("reserve terminal");
    db.reserve_terminal(task_id, attempt.attempt_id, attempt.fence, &reservation, 4)
        .expect("idempotent reserve");
    assert_eq!(
        db.task(task_id).expect("task query").unwrap().status,
        TaskStatus::Committing
    );
    assert_eq!(db.committing_attempts().expect("commits").len(), 1);

    let receipt = json!({"manifest_path": "sources/attempts/x/y/manifest.json"});
    db.finalize_terminal(
        task_id,
        attempt.attempt_id,
        attempt.fence,
        TaskStatus::NeedsReview,
        Some(&receipt),
        5,
    )
    .expect("finalize");
    db.finalize_terminal(
        task_id,
        attempt.attempt_id,
        attempt.fence,
        TaskStatus::NeedsReview,
        Some(&receipt),
        5,
    )
    .expect("idempotent finalize");
    let outbox = db.pending_outbox().expect("outbox");
    assert_eq!(outbox.len(), 5);
    assert_eq!(
        outbox.last().unwrap().payload_json["input_snapshot"],
        "snapshot"
    );
    assert_eq!(db.pending_terminal_audits().unwrap().len(), 1);
    db.mark_terminal_audit_delivered(attempt.attempt_id, 6)
        .unwrap();
    assert!(db.pending_terminal_audits().unwrap().is_empty());
    assert_eq!(
        db.artifact_receipt(task_id, attempt.attempt_id).unwrap(),
        Some(receipt.clone())
    );
    assert!(matches!(
        db.finalize_terminal(
            task_id,
            attempt.attempt_id,
            attempt.fence,
            TaskStatus::Succeeded,
            Some(&receipt),
            6,
        ),
        Err(DbError::InvalidTransition { .. })
    ));
}

#[test]
fn cancelled_attempt_remains_cancelled_after_restart_reconciliation() {
    let temp = TempDir::new().unwrap();
    let db_path = database_path(&temp);
    let vault = VaultUuid::new();
    let task = TaskId::new();
    let attempt = {
        let mut db = ControlDb::open(&db_path, vault).unwrap();
        db.create_task(task, &json!({}), 1).unwrap();
        db.queue_task(task, 2).unwrap();
        let attempt = db
            .claim(task, "worker", snapshot("cancel", vault), 3, 100)
            .unwrap();
        db.cancel_task(task, 4).unwrap();
        assert_eq!(
            db.task(task).unwrap().unwrap().status,
            TaskStatus::CancelRequested
        );
        assert!(
            !db.is_attempt_active(task, attempt.attempt_id, attempt.fence)
                .unwrap()
        );
        attempt
    };
    let mut db = ControlDb::open(&db_path, vault).unwrap();
    assert_eq!(db.abandon_inflight(10).unwrap(), 0);
    assert_eq!(db.cancelling_attempts().unwrap().len(), 1);
    db.acknowledge_cancel(task, attempt.attempt_id, attempt.fence, 11)
        .unwrap();
    db.acknowledge_cancel(task, attempt.attempt_id, attempt.fence, 12)
        .unwrap();
    assert_eq!(
        db.task(task).unwrap().unwrap().status,
        TaskStatus::Cancelled
    );
    assert_eq!(
        db.attempt(task, attempt.attempt_id).unwrap().unwrap().phase,
        AttemptPhase::Cancelled
    );
    assert_eq!(db.abandon_inflight(20).unwrap(), 0);
    assert_eq!(
        db.task(task).unwrap().unwrap().status,
        TaskStatus::Cancelled
    );
}

#[test]
fn every_runtime_transition_enqueues_exactly_the_four_owned_projection_fields() {
    let vault = VaultUuid::new();
    let mut db = ControlDb::open_in_memory(vault).unwrap();
    let task = TaskId::new();
    db.create_task(task, &json!({"body": "authored"}), 1)
        .unwrap();
    db.queue_with_snapshot(task, &snapshot("run-snapshot", vault), 2)
        .unwrap();
    let attempt = db
        .claim(task, "worker", snapshot("run-snapshot", vault), 3, 100)
        .unwrap();
    db.cancel_task(task, 4).unwrap();
    db.acknowledge_cancel(task, attempt.attempt_id, attempt.fence, 5)
        .unwrap();
    let rows = db.pending_outbox().unwrap();
    let statuses: Vec<_> = rows
        .iter()
        .map(|row| row.payload_json["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        statuses,
        [
            "draft",
            "queued",
            "running",
            "cancel_requested",
            "cancelled"
        ]
    );
    for row in rows {
        let mut keys: Vec<_> = row
            .payload_json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "active_attempt",
                "input_snapshot",
                "last_attempt",
                "status",
                "task_id"
            ]
        );
    }
}

#[test]
fn confirmed_run_snapshot_cannot_be_swapped_at_claim() {
    let vault = VaultUuid::new();
    let mut db = ControlDb::open_in_memory(vault).unwrap();
    let task = TaskId::new();
    db.create_task(task, &json!({}), 1).unwrap();
    db.queue_with_snapshot(task, &snapshot("confirmed", vault), 2)
        .unwrap();
    assert!(matches!(
        db.claim(task, "worker", snapshot("replacement", vault), 3, 100),
        Err(DbError::Corrupt(_))
    ));
    assert_eq!(db.task(task).unwrap().unwrap().status, TaskStatus::Queued);
}

#[test]
fn restart_abandons_running_but_preserves_committing_for_roll_forward() {
    let temp = TempDir::new().expect("tempdir");
    let path = database_path(&temp);
    let vault_uuid = VaultUuid::new();
    let running_task = TaskId::new();
    let committing_task = TaskId::new();
    {
        let mut db = ControlDb::open(&path, vault_uuid).expect("open database");
        for (task, title) in [(running_task, "running"), (committing_task, "committing")] {
            db.create_task(task, &json!({"title": title}), 1)
                .expect("create task");
            db.queue_task(task, 2).expect("queue task");
        }
        db.claim(
            running_task,
            "worker",
            snapshot("running-snapshot", vault_uuid),
            3,
            103,
        )
        .expect("running claim");
        let committing = db
            .claim(
                committing_task,
                "worker",
                snapshot("committing-snapshot", vault_uuid),
                4,
                104,
            )
            .expect("committing claim");
        db.reserve_terminal(
            committing_task,
            committing.attempt_id,
            committing.fence,
            &TerminalReservation {
                report_json: json!({"outcome": "needs_review"}),
                artifacts_json: json!([]),
            },
            5,
        )
        .expect("reserve");
    }

    let mut reopened = ControlDb::open(&path, vault_uuid).expect("reopen database");
    let abandoned = reopened.abandon_inflight(200).expect("abandon inflight");
    assert_eq!(abandoned, 1);
    assert_eq!(
        reopened.task(running_task).unwrap().unwrap().status,
        TaskStatus::Abandoned
    );
    assert_eq!(reopened.committing_attempts().unwrap().len(), 1);
}

#[test]
fn snapshot_metadata_excludes_source_content() {
    let temp = TempDir::new().expect("tempdir");
    let vault_uuid = VaultUuid::new();
    let mut db = ControlDb::open(database_path(&temp), vault_uuid).expect("open database");
    let task_id = TaskId::new();
    db.create_task(task_id, &json!({}), 1).unwrap();
    db.queue_task(task_id, 2).unwrap();
    db.claim(
        task_id,
        "worker",
        snapshot("metadata-only", vault_uuid),
        3,
        100,
    )
    .unwrap();

    let stored = db
        .snapshot("metadata-only")
        .expect("snapshot query")
        .unwrap();
    assert_eq!(stored.entries[0].path, "notes/input.md");
    assert_eq!(stored.entries[0].revision, "abc123");
}

#[test]
fn snapshot_identifier_cannot_be_reused_for_different_metadata() {
    let temp = TempDir::new().expect("tempdir");
    let vault_uuid = VaultUuid::new();
    let mut db = ControlDb::open(database_path(&temp), vault_uuid).expect("open database");
    let first = TaskId::new();
    let second = TaskId::new();
    for task in [first, second] {
        db.create_task(task, &json!({}), 1).unwrap();
        db.queue_task(task, 2).unwrap();
    }
    db.claim(first, "worker", snapshot("same-id", vault_uuid), 3, 100)
        .unwrap();

    let mut conflicting = snapshot("same-id", vault_uuid);
    conflicting.entries[0].revision = "different".to_owned();
    assert!(matches!(
        db.claim(second, "worker", conflicting, 4, 101),
        Err(DbError::Corrupt(_))
    ));
}

#[test]
fn acknowledge_cancel_after_a_terminal_winner_is_a_structured_rejection() {
    let vault_uuid = VaultUuid::new();
    let mut db = ControlDb::open_in_memory(vault_uuid).expect("open database");
    let task_id = TaskId::new();
    db.create_task(task_id, &json!({"title": "task"}), 1)
        .expect("create");
    db.queue_task(task_id, 2).expect("queue");
    let attempt = db
        .claim(task_id, "worker", snapshot("snapshot", vault_uuid), 3, 103)
        .expect("claim");
    let reservation = TerminalReservation {
        report_json: json!({"outcome": "needs_review"}),
        artifacts_json: json!([]),
    };
    db.reserve_terminal(task_id, attempt.attempt_id, attempt.fence, &reservation, 4)
        .expect("reserve terminal");

    // 已冻结提交计划：取消确认必须返回结构化终态拒绝，而不是携带状态名的通用拒绝。
    assert!(matches!(
        db.acknowledge_cancel(task_id, attempt.attempt_id, attempt.fence, 5),
        Err(DbError::AlreadyTerminal {
            phase: AttemptPhase::Committing,
            operation: "acknowledge_cancel",
        })
    ));
    db.finalize_terminal(
        task_id,
        attempt.attempt_id,
        attempt.fence,
        TaskStatus::NeedsReview,
        Some(&json!({"manifest_path": "sources/attempts/x/y/manifest.json"})),
        6,
    )
    .expect("finalize");
    assert!(matches!(
        db.acknowledge_cancel(task_id, attempt.attempt_id, attempt.fence, 7),
        Err(DbError::AlreadyTerminal {
            phase: AttemptPhase::Completed,
            operation: "acknowledge_cancel",
        })
    ));
    assert!(matches!(
        db.acknowledge_cancel(task_id, attempt.attempt_id, Fence::new(999).unwrap(), 8),
        Err(DbError::FenceMismatch)
    ));
}

#[test]
fn a_database_written_by_a_newer_schema_is_refused_without_modification() {
    let temp = TempDir::new().expect("tempdir");
    let path = database_path(&temp);
    let vault_uuid = VaultUuid::new();
    drop(ControlDb::open(&path, vault_uuid).expect("initial open"));
    {
        let raw = rusqlite::Connection::open(&path).expect("raw connection");
        raw.execute(
            "INSERT INTO schema_migrations(version, applied_at_ms) VALUES (99, 0)",
            [],
        )
        .expect("record newer version");
    }

    assert!(matches!(
        ControlDb::open(&path, vault_uuid),
        Err(DbError::UnsupportedSchema {
            found: 99,
            supported: 2,
        })
    ));
    // 拒绝不得留下任何写入：更新版记录原样保留，旧二进制没有降级它。
    let raw = rusqlite::Connection::open(&path).expect("raw connection");
    let recorded: i64 = raw
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("query version");
    assert_eq!(recorded, 99);
}
