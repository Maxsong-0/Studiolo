//! SQLite 事务边界、重启后的持久性，以及不带正文的预留。
use serde_json::json;
use std::sync::{Arc, Barrier};
use studiolo_control_db::*;
use studiolo_core::{TaskId, VaultPath, VaultUuid};
fn snapshot(vault: VaultUuid, id: &str) -> SnapshotRecord {
    SnapshotRecord {
        snapshot_id: id.into(),
        vault_uuid: vault,
        entries: vec![],
    }
}
fn definition(path: &str) -> BoardDefinition {
    BoardDefinition {
        acceptance_criteria: vec!["check passes".into()],
        files_owned: vec![VaultPath::parse(path).unwrap()],
        max_children: 8,
        policy_hash: "a".repeat(64),
        parent: None,
        artifact_target: None,
    }
}
#[test]
fn competing_connections_cannot_claim_intersecting_paths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let a = TaskId::new();
    let b = TaskId::new();
    {
        let mut db = ControlDb::open(&path, vault).unwrap();
        for (id, name, owned) in [(a, "a", "src"), (b, "b", "SRC/file.rs")] {
            db.create_board_task(
                name,
                id,
                &json!({"project":"test","adapter":"consumer"}),
                &definition(owned),
                0,
            )
            .unwrap();
            db.queue_task(id, 1).unwrap();
        }
    }
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = [a, b]
        .into_iter()
        .map(|id| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = ControlDb::open(path, vault).unwrap();
                barrier.wait();
                db.claim(
                    id,
                    &id.to_string(),
                    snapshot(vault, &id.to_string()),
                    2,
                    1000,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(DbError::PathsHeld { .. })))
            .count(),
        1
    );
}
#[test]
fn number_allocation_is_atomic_and_survives_restart() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let task = TaskId::new();
    {
        let mut db = ControlDb::open(&path, vault).unwrap();
        db.create_board_task(
            "create",
            task,
            &json!({"project":"test"}),
            &definition("src"),
            0,
        )
        .unwrap();
    }
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|id| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = ControlDb::open(path, vault).unwrap();
                barrier.wait();
                db.reserve_board_number(id, task, "adr", 42).unwrap()
            })
        })
        .collect();
    let mut values: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    values.sort();
    assert_eq!(values, vec![43, 44]);
    let mut db = ControlDb::open(path, vault).unwrap();
    assert_eq!(db.reserve_board_number("next", task, "adr", 5).unwrap(), 45);
}
#[test]
fn expired_lease_cannot_heartbeat_report_or_authorize_gateway_before_sweep() {
    let vault = VaultUuid::new();
    let mut db = ControlDb::open_in_memory(vault).unwrap();
    let task = TaskId::new();
    db.create_task(task, &json!({}), 0).unwrap();
    db.queue_task(task, 1).unwrap();
    let a = db
        .claim(task, "worker", snapshot(vault, "one"), 2, 100)
        .unwrap();
    assert!(
        !db.is_attempt_active_at(task, a.attempt_id, a.fence, 100)
            .unwrap()
    );
    assert!(
        db.heartbeat(task, a.attempt_id, a.fence, 1000, 100)
            .is_err()
    );
    let reservation = TerminalReservation {
        report_json: json!({"outcome":"needs_review"}),
        artifacts_json: json!({"id":"a".repeat(64),"sha256":"b".repeat(64)}),
    };
    assert!(
        db.reserve_terminal(task, a.attempt_id, a.fence, &reservation, 100)
            .is_err()
    );
}
#[test]
fn new_reservation_rejects_inline_artifact_bodies() {
    let vault = VaultUuid::new();
    let mut db = ControlDb::open_in_memory(vault).unwrap();
    let task = TaskId::new();
    db.create_task(task, &json!({}), 0).unwrap();
    db.queue_task(task, 1).unwrap();
    let a = db
        .claim(task, "worker", snapshot(vault, "one"), 2, 100)
        .unwrap();
    let reservation = TerminalReservation {
        report_json: json!({"outcome":"needs_review"}),
        artifacts_json: json!([{"name":"x.md","content":[1,2,3]}]),
    };
    assert!(
        db.reserve_terminal(task, a.attempt_id, a.fence, &reservation, 3)
            .is_err()
    );
    assert_eq!(
        db.attempt(task, a.attempt_id).unwrap().unwrap().phase,
        AttemptPhase::Running
    );
}
#[test]
fn restart_revokes_live_board_attempt_and_keeps_auditable_reason() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let task = TaskId::new();
    {
        let mut db = ControlDb::open(&path, vault).unwrap();
        db.create_board_task(
            "create",
            task,
            &json!({"project":"test","adapter":"consumer"}),
            &definition("src"),
            0,
        )
        .unwrap();
        db.queue_task(task, 1).unwrap();
        db.claim(task, "worker", snapshot(vault, "one"), 2, 1000)
            .unwrap();
    }
    let mut db = ControlDb::open(path, vault).unwrap();
    assert_eq!(
        db.reconcile_leases(3, true).unwrap().abandoned[0].2,
        "restart"
    );
    assert_eq!(
        db.task(task).unwrap().unwrap().status,
        TaskStatus::Abandoned
    );
    assert!(!db.pending_runtime_audits().unwrap().is_empty());
    assert!(db.reconcile_leases(4, true).unwrap().abandoned.is_empty());
}
#[test]
fn lease_plan_is_a_dry_run_and_a_database_changed_since_voids_it() {
    let vault = VaultUuid::new();
    let mut db = ControlDb::open_in_memory(vault).unwrap();
    let first = TaskId::new();
    db.create_task(first, &json!({"adapter":"consumer"}), 0)
        .unwrap();
    db.queue_task(first, 1).unwrap();
    let a = db
        .claim(first, "worker", snapshot(vault, "one"), 2, 100)
        .unwrap();
    // claim 自己也会留待投的运行时结果；预演前后条数不变才算没留改动。
    let pending = db.pending_runtime_audits().unwrap().len();
    let plan = db.plan_lease_reconciliation(200, false).unwrap();
    assert_eq!(
        plan,
        vec![LeaseStep {
            task_id: first,
            attempt_id: a.attempt_id,
            action: LeaseAction::Abandon,
            reason: "lease_expired".into(),
        }]
    );
    assert_eq!(
        db.task(first).unwrap().unwrap().status,
        TaskStatus::Running,
        "预演不留改动"
    );
    assert_eq!(db.pending_runtime_audits().unwrap().len(), pending);
    // 预演之后又有一个租约到期：照旧清单执行会多做一步意图里没写的事，所以整轮不做。
    let second = TaskId::new();
    db.create_task(second, &json!({"adapter":"consumer"}), 3)
        .unwrap();
    db.queue_task(second, 4).unwrap();
    db.claim(second, "worker", snapshot(vault, "two"), 5, 150)
        .unwrap();
    let pending = db.pending_runtime_audits().unwrap().len();
    assert_eq!(db.apply_lease_plan(200, false, &plan).unwrap(), None);
    assert_eq!(db.task(first).unwrap().unwrap().status, TaskStatus::Running);
    assert_eq!(db.pending_runtime_audits().unwrap().len(), pending);
    let plan = db.plan_lease_reconciliation(200, false).unwrap();
    assert_eq!(plan.len(), 2);
    let done = db.apply_lease_plan(200, false, &plan).unwrap().unwrap();
    assert_eq!(done.abandoned.len(), 2);
    assert_eq!(
        db.task(first).unwrap().unwrap().status,
        TaskStatus::Abandoned
    );
    assert!(db.plan_lease_reconciliation(300, false).unwrap().is_empty());
}
#[test]
fn a_restored_control_reopens_without_migrating_and_revokes_every_live_attempt() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let task = TaskId::new();
    let expected = {
        let mut db = ControlDb::open(&path, vault).unwrap();
        db.create_task(task, &json!({"adapter":"consumer"}), 0)
            .unwrap();
        db.queue_task(task, 1).unwrap();
        db.claim(task, "worker", snapshot(vault, "one"), 2, 1_000_000)
            .unwrap();
        // 封存前各 outbox 都已排空：这时的水位就是清单里会记的那一份。
        for record in db.pending_runtime_audits().unwrap() {
            db.acknowledge_runtime_audit(&record.event_id).unwrap();
        }
        for record in db.pending_outbox().unwrap() {
            db.mark_outbox_delivered(record.sequence, 3).unwrap();
        }
        db.restored_watermark().unwrap()
    };
    // 只开已有库：Vault 绑定不同、目录里没有库，都拒绝，也不建出新文件。
    assert!(ControlDb::open_existing(temp.path(), VaultUuid::new()).is_err());
    let empty = tempfile::tempdir().unwrap();
    assert!(ControlDb::open_existing(empty.path(), vault).is_err());
    assert_eq!(std::fs::read_dir(empty.path()).unwrap().count(), 0);

    let mut db = ControlDb::open_existing(temp.path(), vault).unwrap();
    assert_eq!(db.restored_watermark().unwrap(), expected);
    assert_eq!(
        db.live_attempts().unwrap(),
        LiveAttempts {
            leased: 1,
            cancel_requested: 0
        }
    );
    // 租约远没到期，启动巡检照样撤销；撤销之后再来一遍什么都不变。
    assert_eq!(db.reconcile_leases(10, true).unwrap().abandoned.len(), 1);
    assert_eq!(db.live_attempts().unwrap().leased, 0);
    assert!(db.reconcile_leases(11, true).unwrap().abandoned.is_empty());
    assert_eq!(
        db.task(task).unwrap().unwrap().status,
        TaskStatus::Abandoned
    );
    // 撤销留下了待投的运行时结果，库已经不是封存那一份：水位核对不再相等。
    assert!(db.restored_watermark().is_err());
}
