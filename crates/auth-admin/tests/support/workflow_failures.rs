//! 真实权限故障、关闭重开和持久限流；失败后的原决定只能补记，不能恢复激活。

use super::*;
use std::time::Instant;

#[test]
fn invalid_terminal_input_never_enters_enrollment_or_consumes_the_auth_budget() {
    let f = Fixture::new();
    f.initialize();
    for _ in 0..6 {
        assert!(matches!(
            f.run(Command::Enroll, || Err(TerminalError::InvalidCode)),
            Err(AdminError::Terminal(TerminalError::InvalidCode))
        ));
        assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
    }
    assert!(f.events("totp.enrolled").is_empty());
    f.run(Command::Enroll, current_code).unwrap();
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Enrolled);
}

#[test]
fn wrong_codes_exhaust_persistent_budget_across_each_reopen_and_audit_flush() {
    let f = Fixture::new();
    f.initialize();
    for _ in 0..5 {
        assert!(matches!(
            f.run(Command::Enroll, wrong_code),
            Err(AdminError::Session(SessionError::Rejected))
        ));
        assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
    }
    assert!(matches!(
        f.run(Command::Enroll, current_code),
        Err(AdminError::Session(SessionError::RateLimited))
    ));
    f.run(Command::AuditFlush, never_prompt).unwrap();
    assert!(matches!(
        f.run(Command::Enroll, current_code),
        Err(AdminError::Session(SessionError::RateLimited))
    ));
    assert!(f.events("totp.enrolled").is_empty());
    assert!(!f.events("session.denied").is_empty());
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
}

#[test]
fn real_audit_permission_failure_leaves_pending_and_flush_cannot_enroll() {
    let f = Fixture::new();
    f.initialize();
    let started = Instant::now();
    let database = f.audit.join("audit.sqlite3");
    let failed = f.run(Command::Enroll, || {
        // Ledger 已合法打开；临时撤去写权限后，Auth 消费仍可提交而审计检查拒绝。
        mode(&database, 0o400);
        current_code()
    });
    mode(&database, 0o600);
    assert!(matches!(
        failed,
        Err(AdminError::Session(SessionError::AuditUnavailable))
    ));
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Pending);
    assert!(f.events("totp.enrolled").is_empty());
    f.run(Command::AuditFlush, never_prompt).unwrap();
    // 本用例不等待五分钟，也不改时钟；若执行环境长暂停，明确标出夹具期限失效。
    assert!(
        started.elapsed().as_secs() < 300,
        "测试执行超过真实 pending 窗口"
    );
    assert_eq!(f.events("totp.enrolled").len(), 2);
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Pending);
    assert!(matches!(
        f.run(Command::Enroll, never_prompt),
        Err(AdminError::Session(SessionError::OperationPending))
    ));
    f.run(Command::AuditFlush, never_prompt).unwrap();
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Pending);
    assert_eq!(f.events("totp.enrolled").len(), 2);
    assert!(f.events("totp.step_up").is_empty());
}
