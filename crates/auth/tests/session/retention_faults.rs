//! 清理许可不是已删除结果：审计前后和最终 Auth 写失败都必须保留原计划。

use super::signed_identity::stored_text;
use super::*;
use std::sync::{Arc, atomic::Ordering};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};

fn retired(h: &Harness) -> IssuedSession {
    let issued = h.manager.issue(&identity()).unwrap();
    assert_eq!(
        h.manager
            .revoke_all(&identity(), &issued.token, &issued.csrf)
            .unwrap()
            .revoked,
        1
    );
    issued
}

fn checkpoint(manager: &SessionManager) -> Vec<String> {
    stored_text(
        manager,
        "SELECT checkpoint_json FROM auth_retention_checkpoint",
    )
}

#[test]
fn checkpoint_append_before_or_after_failure_resumes_same_plan_after_reopen() {
    for committed_to_audit in [false, true] {
        let h = Harness::new(10, 100);
        let old = retired(&h);
        let before = checkpoint(&h.manager);
        let weak = Arc::downgrade(&h.audit);
        *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
            if event.action() == "session.compacted" {
                let audit = weak.upgrade().unwrap();
                if committed_to_audit {
                    audit.after_append.store(true, Ordering::SeqCst);
                } else {
                    audit.fail.store(true, Ordering::SeqCst);
                }
            }
        }));
        assert_eq!(
            h.manager.maintain_retention().unwrap_err(),
            SessionError::AuditUnavailable
        );
        assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 1);
        assert_eq!(
            scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
            1
        );
        assert_eq!(checkpoint(&h.manager), before);
        let plan = stored_text(&h.manager, "SELECT plan_json FROM auth_retention_plan");
        let filter = AuditFilter::default().with_action("session.compacted");
        let original_permit = h.audit.query(&filter).unwrap();
        assert_eq!(original_permit.len(), usize::from(committed_to_audit));

        // 重新打开不依赖审计恢复，也不能擅自丢掉未完成计划。
        let reopened = h.reopen(10, 100);
        assert_eq!(
            stored_text(&reopened, "SELECT plan_json FROM auth_retention_plan"),
            plan
        );
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 1);
        *h.audit.hook.lock().unwrap() = None;
        h.audit.fail.store(false, Ordering::SeqCst);
        h.audit.after_append.store(false, Ordering::SeqCst);
        assert_eq!(reopened.maintain_retention().unwrap().reclaimed_sessions, 1);
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 0);
        assert_eq!(
            scalar(&reopened, "SELECT count(*) FROM auth_retention_plan"),
            0
        );
        let permits = h.audit.query(&filter).unwrap();
        assert_eq!(permits.len(), 1);
        if committed_to_audit {
            assert_eq!(permits, original_permit);
        }
        assert_eq!(reopened.maintain_retention().unwrap().reclaimed_sessions, 0);
        assert_eq!(
            reopened.check(&identity(), &old.token).unwrap_err(),
            SessionError::Rejected
        );
    }
}

#[test]
fn final_auth_write_failure_keeps_permission_and_rows_for_atomic_retry() {
    let h = Harness::new(10, 100);
    retired(&h);
    let before = checkpoint(&h.manager);
    let weak = Arc::downgrade(&h.manager);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.compacted" {
            weak.upgrade()
                .unwrap()
                .database
                .connection
                .lock()
                .unwrap()
                .execute_batch("PRAGMA query_only=ON")
                .unwrap();
        }
    }));
    assert_eq!(
        h.manager.maintain_retention().unwrap_err(),
        SessionError::StorageUnavailable
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 1);
    assert_eq!(
        scalar(&h.manager, "SELECT count(*) FROM auth_retention_plan"),
        1
    );
    assert_eq!(checkpoint(&h.manager), before);
    let filter = AuditFilter::default().with_action("session.compacted");
    let permits = h.audit.query(&filter).unwrap();
    assert_eq!(permits.len(), 1);
    *h.audit.hook.lock().unwrap() = None;
    h.manager
        .database
        .connection
        .lock()
        .unwrap()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    let reopened = h.reopen(10, 100);
    assert_eq!(reopened.maintain_retention().unwrap().reclaimed_sessions, 1);
    assert_eq!(h.audit.query(&filter).unwrap(), permits);
    assert_eq!(scalar(&reopened, "SELECT count(*) FROM sessions"), 0);
    assert_eq!(
        scalar(&reopened, "SELECT count(*) FROM auth_retention_plan"),
        0
    );
    assert_ne!(checkpoint(&reopened), before);
}
