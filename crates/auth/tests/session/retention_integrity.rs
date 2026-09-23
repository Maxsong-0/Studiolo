//! 只为损坏反例直写测试控制表；真实长期签发由 longevity 独立完成。

use super::signed_identity::stored_text;
use super::*;
use std::sync::{Arc, atomic::Ordering};

fn numeric_change(encoded: &str, field: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(encoded).unwrap();
    let original = value[field].as_i64().unwrap();
    let before = format!("\"{field}\":{original}");
    let after = format!("\"{field}\":{}", original + 1);
    assert!(encoded.contains(&before));
    // 保持其余字节和字段顺序，避免仅靠非规范 JSON 排序使反例失败。
    encoded.replacen(&before, &after, 1)
}

fn assert_rejected_before_any_deletion(h: &Harness, controller: &IssuedSession, rows: i64) {
    assert_eq!(
        h.manager.check(&identity(), &controller.token).unwrap_err(),
        SessionError::CorruptStore
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap_err(),
        SessionError::CorruptStore
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), rows);
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_checkpoint"),
        1
    );
}

#[test]
fn checkpoint_single_field_changes_are_rejected_by_first_read_and_maintenance() {
    for field in [
        "sequence",
        "previous_sequence",
        "epoch_base",
        "issuance_floor",
        "event_sequence_barrier",
        "reclaimed_sessions",
    ] {
        let h = Harness::new(10, 100);
        let controller = h.manager.issue(&identity()).unwrap();
        let target = h.manager.issue(&identity()).unwrap();
        h.manager
            .revoke(
                &identity(),
                &controller.token,
                &controller.csrf,
                &target.session.id,
            )
            .unwrap();
        assert_eq!(
            h.manager.maintain_retention().unwrap().reclaimed_sessions,
            1
        );
        let original = stored_text(
            &h.manager,
            "SELECT checkpoint_json FROM auth_retention_checkpoint",
        );
        let changed = numeric_change(&original[0], field);
        h.manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE auth_retention_checkpoint SET checkpoint_json=?1 WHERE id=1",
                [&changed],
            )
            .unwrap();
        assert_rejected_before_any_deletion(&h, &controller, 1);
    }
}

#[test]
fn persisted_plan_single_field_or_manifest_damage_cannot_authorize_deletion() {
    for change_manifest in [false, true] {
        let h = Harness::new(10, 100);
        let controller = h.manager.issue(&identity()).unwrap();
        let target = h.manager.issue(&identity()).unwrap();
        h.manager
            .revoke(
                &identity(),
                &controller.token,
                &controller.csrf,
                &target.session.id,
            )
            .unwrap();
        let weak = Arc::downgrade(&h.audit);
        *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "session.compacted" {
                weak.upgrade().unwrap().fail.store(true, Ordering::SeqCst);
            }
        }));
        assert_eq!(
            h.manager.maintain_retention().unwrap_err(),
            SessionError::AuditUnavailable
        );
        *h.audit.hook.lock().unwrap() = None;
        h.audit.fail.store(false, Ordering::SeqCst);
        assert_eq!(
            scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
            1
        );
        if change_manifest {
            let original = stored_text(&h.manager, "SELECT manifest_hash FROM auth_retention_plan");
            let replacement = if original[0].starts_with('a') {
                "b"
            } else {
                "a"
            };
            let changed = format!("{replacement}{}", &original[0][1..]);
            h.manager
                .database
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE auth_retention_plan SET manifest_hash=?1 WHERE id=1",
                    [&changed],
                )
                .unwrap();
        } else {
            let original = stored_text(&h.manager, "SELECT plan_json FROM auth_retention_plan");
            let changed = numeric_change(&original[0], "issuance_floor");
            h.manager
                .database
                .connection
                .lock()
                .unwrap()
                .execute(
                    "UPDATE auth_retention_plan SET plan_json=?1 WHERE id=1",
                    [&changed],
                )
                .unwrap();
        }
        assert_rejected_before_any_deletion(&h, &controller, 2);
        assert_eq!(
            scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
            1
        );
    }
}

fn reopen_error(h: &Harness) -> Option<SessionError> {
    SessionManager::open(
        &h.root,
        config(10, 100),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    )
    .err()
}

#[test]
fn missing_checkpoint_anchor_row_is_corrupt_store_not_storage_unavailable() {
    // 单例锚点按不变量必然存在；缺行是库损坏，不能被误报成可重试的存储不可用。
    let h = Harness::new(10, 100);
    let controller = h.manager.issue(&identity()).unwrap();
    let deleted = h
        .manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute("DELETE FROM auth_retention_checkpoint WHERE id=1", [])
        .unwrap();
    assert_eq!(deleted, 1);
    assert_eq!(
        h.manager.check(&identity(), &controller.token).unwrap_err(),
        SessionError::CorruptStore
    );
    assert_eq!(
        h.manager.maintain_retention().unwrap_err(),
        SessionError::CorruptStore
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(reopen_error(&h), Some(SessionError::CorruptStore));
}

#[test]
fn missing_paired_revoke_outbox_row_is_corrupt_store_not_storage_unavailable() {
    // 吊销的 intent 与 outcome 同事务成对写入；删掉任一关联行都只能判为库损坏。
    for phase in ["intent", "outcome"] {
        let h = Harness::new(10, 100);
        let controller = h.manager.issue(&identity()).unwrap();
        let target = h.manager.issue(&identity()).unwrap();
        h.manager
            .revoke(
                &identity(),
                &controller.token,
                &controller.csrf,
                &target.session.id,
            )
            .unwrap();
        let deleted = h
            .manager
            .database
            .connection
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM audit_outbox WHERE kind='revoke' AND phase=?1",
                [phase],
            )
            .unwrap();
        assert_eq!(deleted, 1);
        assert_rejected_before_any_deletion(&h, &controller, 2);
        assert_eq!(reopen_error(&h), Some(SessionError::CorruptStore));
    }
}
