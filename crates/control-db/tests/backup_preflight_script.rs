//! 上线前只读核查脚本（`scripts/check-backup-preflight.py`）对着真实控制库跑：
//! 脚本说「会挡封存」的，检查点冻结一定拒绝；脚本说干净的，冻结一定通过。脚本不建也不改库旁边的文件。
#![cfg(unix)]
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_control_db::{ControlDb, SnapshotRecord, TaskStatus, TerminalReservation};
use studiolo_core::{Revision, TaskId, VaultPath, VaultUuid};

const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../scripts/check-backup-preflight.py"
);

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

/// 跑一次脚本：退出码，和发现的类别（去重排序）。
fn preflight(control: &Path) -> (i32, Vec<String>) {
    let output = Command::new("python3")
        .args(["-I", "-B", SCRIPT, "--json", "--control"])
        .arg(control)
        .output()
        .expect("python3 可用");
    let code = output.status.code().expect("脚本正常退出");
    assert!(
        code == 0 || code == 1,
        "脚本没做完：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut kinds: Vec<String> = report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|finding| finding["kind"].as_str().unwrap().to_owned())
        .collect();
    kinds.sort();
    kinds.dedup();
    (code, kinds)
}

/// 库目录里有哪些名字，主库文件和 -wal 的长度和修改时间（-shm 是读写双方的共享内存，只看在不在）。
fn listing(directory: &Path) -> BTreeSet<(String, Option<(u64, i128)>)> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let metadata = entry.metadata().unwrap();
            let stamp = (!name.ends_with("-shm")).then(|| {
                let modified = metadata
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap();
                (metadata.len(), modified.as_nanos() as i128)
            });
            (name, stamp)
        })
        .collect()
}

#[test]
fn script_and_checkpoint_freeze_agree_on_a_real_control_database() {
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    // 审计账本只开属主私有的目录（和 checkpoint_contract.rs 的夹具一样）。
    let audit = tempfile::tempdir().unwrap();
    fs::set_permissions(audit.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.path().join("control.sqlite3");
    fs::File::create(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let vault = VaultUuid::new();
    let mut db = ControlDb::open_checkpoint_ready(directory.path(), vault).unwrap();
    let source = db.checkpoint_source().unwrap();
    let ledger = SqliteAuditLedger::open(audit.path()).unwrap();

    // 一个 attempt 停在「提交中」。
    let task = TaskId::new();
    db.create_task(task, &json!({"title":"preflight"}), 1)
        .unwrap();
    db.queue_task(task, 2).unwrap();
    let attempt = db
        .claim(
            task,
            "preflight-worker",
            SnapshotRecord {
                snapshot_id: "fixed".into(),
                vault_uuid: vault,
                entries: vec![],
            },
            3,
            103,
        )
        .unwrap();
    let reservation = TerminalReservation {
        report_json: json!({"format":3,"event_id":"preflight-terminal","task_id":task,"attempt_id":attempt.attempt_id,"fence":attempt.fence}),
        artifacts_json: json!({"id":"a".repeat(64),"sha256":"b".repeat(64)}),
    };
    db.reserve_terminal(task, attempt.attempt_id, attempt.fence, &reservation, 4)
        .unwrap();
    let before = listing(directory.path());
    let (code, kinds) = preflight(&path);
    assert_eq!(code, 1);
    assert!(
        kinds.contains(&"control_committing".to_owned()),
        "{kinds:?}"
    );
    assert_eq!(
        listing(directory.path()),
        before,
        "脚本不能建或改库旁边的文件"
    );
    assert!(source.freeze(deadline()).is_err());

    // 提交完、审计都投递：两边都通过。
    let receipt = json!({"manifest_path":VaultPath::parse("artifacts/preflight.json").unwrap(),"manifest_revision":Revision::from_bytes([7;32]),"artifacts":[]});
    db.finalize_terminal(
        task,
        attempt.attempt_id,
        attempt.fence,
        TaskStatus::NeedsReview,
        Some(&receipt),
        5,
    )
    .unwrap();
    for row in db.pending_outbox().unwrap() {
        db.mark_outbox_delivered(row.sequence, 100).unwrap();
    }
    for outcome in db.pending_terminal_audits().unwrap() {
        ledger.append(outcome.audit_event().unwrap()).unwrap();
        db.mark_terminal_audit_delivered(outcome.attempt.attempt_id, 6)
            .unwrap();
    }
    for event in db.pending_runtime_audits().unwrap() {
        ledger.append(event.audit_event().unwrap()).unwrap();
        db.acknowledge_runtime_audit(&event.event_id).unwrap();
    }
    assert_eq!(preflight(&path), (0, vec![]));
    drop(source.freeze(deadline()).expect("干净的库应该能冻结"));

    // 旧格式的终态（旧内联预留：产物是数组）：两边都拒绝；改回来两边都恢复。
    let raw = rusqlite::Connection::open(&path).unwrap();
    let id = attempt.attempt_id.to_string();
    let original: Option<String> = raw
        .query_row(
            "SELECT terminal_json FROM attempts WHERE attempt_id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    raw.execute(
        "UPDATE attempts SET terminal_json=json_object('report_json',json_object('format',3),'artifacts_json',json('[]')) WHERE attempt_id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(
        preflight(&path),
        (1, vec!["control_terminal_shape".to_owned()])
    );
    assert!(source.freeze(deadline()).is_err());
    raw.execute(
        "UPDATE attempts SET terminal_json=?1 WHERE attempt_id=?2",
        rusqlite::params![original, id],
    )
    .unwrap();
    assert_eq!(preflight(&path), (0, vec![]));
    drop(source.freeze(deadline()).expect("改回来以后应该能冻结"));

    // 主服务停了（连接全关，-wal、-shm 被收掉）：照样读得出，也不会把它们建出来。
    drop(raw);
    drop(source);
    drop(db);
    let before = listing(directory.path());
    assert!(
        !before
            .iter()
            .any(|(name, _)| name.ends_with("-shm") || name.ends_with("-wal"))
    );
    assert_eq!(preflight(&path), (0, vec![]));
    assert_eq!(
        listing(directory.path()),
        before,
        "脚本不能建或改库旁边的文件"
    );
}
