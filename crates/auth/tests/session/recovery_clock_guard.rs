//! 外部坏历史不是 AuthDB 损坏；拒绝仍须提交已观察时间和会话失效。

use super::*;
use std::sync::Arc;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, MemoryAuditLedger,
};

struct AdvancingHistory {
    real: Arc<FaultAudit>,
    foreign: MemoryAuditLedger,
    original: AuditRecord,
    clock: Arc<TestClock>,
    late: i64,
}

impl AuditLedger for AdvancingHistory {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.event_id() != self.original.event().event_id()
            || event.phase() != self.original.event().phase()
        {
            return self.real.append(event);
        }
        // 同一原事件由另一真实哈希链产生回执；推进时钟模拟审计等待而不使用 sleep。
        self.clock.set(self.late);
        let record = self.foreign.append(event.clone())?;
        record.verify_integrity().unwrap();
        assert_eq!(record.event(), &event);
        assert_ne!(record, self.original);
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.real.query(filter)
    }
}

fn faulty_manager(h: &RecoveryHarness, late: i64) -> Arc<SessionManager> {
    let records = h.h.audit.query(&AuditFilter::default()).unwrap();
    let original = records
        .iter()
        .find(|record| record.event().action() == "totp.enrolled")
        .unwrap()
        .clone();
    let foreign = MemoryAuditLedger::new();
    // 独立前缀确保原登记不会意外获得同一 genesis、offset 和哈希。
    foreign
        .append(records.last().unwrap().event().clone())
        .unwrap();
    let audit = Arc::new(AdvancingHistory {
        real: h.h.audit.clone(),
        foreign,
        original,
        clock: h.h.clock.clone(),
        late,
    });
    Arc::new(
        SessionManager::open(
            &h.h.root,
            config(600, 7200),
            audit,
            h.h.clock.clone(),
            h.h.random.clone(),
        )
        .unwrap(),
    )
}

#[derive(Clone, Copy)]
enum Attempt {
    Rotate,
    Replace,
    StepUp,
}

fn assert_clock_guard(attempt: Attempt) {
    for expire_l2 in [false, true] {
        let h = RecoveryHarness::new(600, 7200);
        let (issued, codes) = h.ready();
        let prepared = h.prepared();
        let owner = identity();
        let req = request(&owner, &issued, 1);
        // 第一轮仅越过旧 L3 期限，第二轮同时越过最近认证请求后的 L2 闲置期限。
        let late = if expire_l2 {
            CHANGE + 600_000
        } else {
            NEXT + 300_001
        };
        h.h.clock.set(CHANGE);
        let manager = faulty_manager(&h, late);
        let recovery = Recovery::attach(manager.clone()).unwrap();
        let (totp, _) = Totp::attach(manager.clone(), seed(OLD)).unwrap();
        let result = match attempt {
            Attempt::Rotate => recovery.rotate_codes(&req).map(|_| ()),
            Attempt::Replace => recovery
                .replace_factor(&req, copy_code(&codes, 0), &prepared, otp(&NEW, CHANGE))
                .map(|_| ()),
            Attempt::StepUp => totp.step_up(&req, otp(&OLD, CHANGE)).map(|_| ()),
        };
        assert_eq!(result.unwrap_err(), SessionError::AuditUnavailable);
        assert_eq!(scalar(&manager, "SELECT clock_floor FROM auth_meta"), late);
        assert_eq!(
            scalar(&manager, "SELECT total FROM auth_denials WHERE kind=6"),
            1
        );
        assert_eq!(
            scalar(
                &manager,
                "SELECT count(*) FROM sessions WHERE state='expired'"
            ),
            i64::from(expire_l2)
        );

        // 恢复权威 Adapter 并完整重开；错误外部回执不能把健康存储标成损坏。
        let (reopened, healthy_recovery) = h.reopen();
        let (healthy_totp, _) = Totp::attach(reopened.clone(), seed(OLD)).unwrap();
        h.h.clock.set(CHANGE);
        assert_eq!(
            reopened.check(&owner, &issued.token).unwrap_err(),
            SessionError::ClockRollback
        );
        assert_eq!(
            healthy_totp
                .require_l3(&req, SensitiveAction::ManageRecoveryCodes)
                .unwrap_err(),
            SessionError::ClockRollback
        );
        assert_eq!(scalar(&reopened, "SELECT clock_floor FROM auth_meta"), late);

        h.h.clock.set(late);
        assert_eq!(
            healthy_totp
                .require_l3(&req, SensitiveAction::ManageRecoveryCodes)
                .unwrap_err(),
            if expire_l2 {
                SessionError::Rejected
            } else {
                SessionError::ReauthenticationRequired
            }
        );
        // 双确认后只清理失效决定；旧批次、已消费码和旧因子不会被补记重新发布。
        h.h.clock.set(late.max(CHANGE + 300_000));
        match attempt {
            Attempt::StepUp => assert!(healthy_totp.flush_audit().unwrap() <= 2),
            _ => assert!(healthy_recovery.flush_audit().unwrap() <= 2),
        }
        assert_eq!(
            scalar(&reopened, "SELECT count(*) FROM recovery_pending"),
            0
        );
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM totp_pending"), 0);
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM recovery_codes"), 10);
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM totp_grants"), 1);
        assert_eq!(
            scalar(&reopened, "SELECT used_mask FROM recovery_control"),
            i64::from(matches!(attempt, Attempt::Replace))
        );
        assert!(Totp::attach(reopened, seed(OLD)).is_ok());
    }
}

#[test]
fn rotation_bad_history_commits_clock_floor_before_returning_audit_failure() {
    assert_clock_guard(Attempt::Rotate);
}

#[test]
fn replacement_bad_history_commits_clock_floor_without_reviving_consumed_code() {
    assert_clock_guard(Attempt::Replace);
}

#[test]
fn totp_bad_history_commits_clock_floor_without_reviving_previous_l3() {
    assert_clock_guard(Attempt::StepUp);
}
