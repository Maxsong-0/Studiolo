//! 通过 CLI 的执行 Interface 验收，不复制 Auth 私有 SQL 或构造 Access 身份。

#[path = "workflow_failures.rs"]
mod failures;
#[path = "workflow_support.rs"]
mod support;

use super::*;
use std::{cell::Cell, fs};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{SessionError, TotpEnrollmentState};
use support::*;

#[test]
fn invalid_command_or_missing_seed_cannot_touch_either_empty_store() {
    let f = Fixture::new();
    for command in [Command::Enroll, Command::AuditFlush] {
        assert!(matches!(
            execute(command, f.load(), None, never_prompt),
            Err(AdminError::Environment)
        ));
        f.empty();
    }
    for step in [0, 4, 255] {
        assert!(matches!(
            execute(Command::Migrate(step), f.load(), None, never_prompt),
            Err(AdminError::Arguments)
        ));
        f.empty();
    }
}

#[test]
fn ordinary_commands_never_create_missing_audit_or_auth_database() {
    let f = Fixture::new();
    for command in [Command::Enroll, Command::AuditFlush] {
        assert!(matches!(
            f.run(command, never_prompt),
            Err(AdminError::MissingAudit)
        ));
        f.empty();
    }
    drop(SqliteAuditLedger::open(&f.audit).unwrap());
    for command in [Command::Enroll, Command::AuditFlush] {
        assert!(matches!(
            f.run(command, never_prompt),
            Err(AdminError::Session(SessionError::MissingStore))
        ));
        assert_eq!(fs::read_dir(&f.auth).unwrap().count(), 0);
        assert!(f.audit.join("audit.sqlite3").is_file());
    }
}

#[test]
fn explicit_migration_prepares_storage_without_requesting_otp_or_enrolling() {
    let f = Fixture::new();
    f.initialize();
    assert!(f.auth.join("auth.sqlite3").is_file());
    assert!(f.audit.join("audit.sqlite3").is_file());
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
    assert!(f.events("totp.enrolled").is_empty());
    assert!(f.events("session.issue").is_empty());
    assert!(f.events("totp.step_up").is_empty());
    f.run(Command::AuditFlush, never_prompt).unwrap();
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
}

#[test]
fn older_explicit_steps_do_not_become_implicit_migration_during_enrollment() {
    let f = Fixture::new();
    f.run(Command::Migrate(1), never_prompt).unwrap();
    assert!(matches!(
        f.run(Command::Enroll, never_prompt),
        Err(AdminError::Session(SessionError::MigrationRequired))
    ));
    f.run(Command::Migrate(2), never_prompt).unwrap();
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
    f.run(Command::Migrate(3), never_prompt).unwrap();
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Ready);
    assert!(f.events("totp.enrolled").is_empty());
}

#[test]
fn enrollment_commits_once_and_reopened_duplicate_is_rejected_before_prompt() {
    let f = Fixture::new();
    f.initialize();
    let prompts = Cell::new(0);
    f.run(Command::Enroll, || {
        prompts.set(prompts.get() + 1);
        current_code()
    })
    .unwrap();
    assert_eq!(prompts.get(), 1);
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Enrolled);
    let before = f.events("totp.enrolled");
    assert_eq!(before.len(), 2);
    for _ in 0..2 {
        assert!(matches!(
            f.run(Command::Enroll, never_prompt),
            Err(AdminError::Session(SessionError::AlreadyEnrolled))
        ));
        assert_eq!(f.events("totp.enrolled"), before);
    }
    // 幂等迁移不意味着清除已有因子；旧登记不会被本次命令再次宣布或创建。
    f.run(Command::Migrate(3), never_prompt).unwrap();
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Enrolled);
    assert_eq!(f.events("totp.enrolled"), before);
    assert!(f.events("totp.step_up").is_empty());
}

#[test]
fn changed_seed_binding_is_rejected_before_requesting_any_otp() {
    let f = Fixture::new();
    f.initialize();
    f.run(Command::Enroll, current_code).unwrap();
    for command in [Command::Enroll, Command::AuditFlush] {
        assert!(matches!(
            execute(command, f.load(), Some(other_seed()), never_prompt),
            Err(AdminError::Session(SessionError::Configuration))
        ));
    }
    assert_eq!(f.state().unwrap(), TotpEnrollmentState::Enrolled);
    assert_eq!(f.events("totp.enrolled").len(), 2);
}
