//! 产品进程必须拒绝不完整装配；不以私有 SQL 注入成功或失败业务状态。

use super::fixture::{AUDIENCE, EMAIL, Fixture, ISSUER, ORIGIN};
use serde_json::json;
use std::fs;
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_auth::{SessionError, TotpEnrollmentState};

#[test]
fn missing_both_stores_creates_nothing() {
    let fixture = Fixture::new();
    fixture.reject(fixture.args());
    fixture.empty_stores();
}

#[test]
fn missing_auth_is_not_initialized_when_audit_exists() {
    let fixture = Fixture::new();
    drop(SqliteAuditLedger::open(&fixture.audit).unwrap());
    fixture.reject(fixture.args());
    assert_eq!(fs::read_dir(&fixture.auth).unwrap().count(), 0);
    assert!(
        SqliteAuditLedger::open_existing(&fixture.audit)
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn missing_audit_is_not_initialized_when_auth_exists() {
    let fixture = Fixture::new();
    fixture.initialize(4, true, false);
    let missing = fixture.root.join("empty-audit");
    fs::create_dir(&missing).unwrap();
    super::fixture::mode(&missing, 0o700);
    let mut value = fixture.value();
    value["audit_directory"] = json!(missing);
    fixture.write_config(&value);
    fixture.reject(fixture.args());
    assert_eq!(fs::read_dir(missing).unwrap().count(), 0);
}

#[test]
fn unregistered_v4_cannot_start() {
    let fixture = Fixture::new();
    fixture.initialize(4, false, false);
    fixture.reject(fixture.args());
}

#[test]
fn pending_first_enrollment_cannot_start_or_self_activate() {
    let fixture = Fixture::new();
    fixture.initialize(4, false, true);
    fixture.reject(fixture.args());
    let events = SqliteAuditLedger::open_existing(&fixture.audit)
        .unwrap()
        .query(&AuditFilter::default())
        .unwrap();
    assert!(events.is_empty(), "启动不能补记或激活首次登记");
    assert_eq!(
        fixture.enrollment_state().unwrap(),
        TotpEnrollmentState::Pending
    );
}

#[test]
fn v2_is_not_silently_migrated_for_http() {
    let fixture = Fixture::new();
    fixture.initialize(2, false, false);
    fixture.reject(fixture.args());
    // 合法打开可推进时间地板；通过因子 Interface 验证仍未迁移，而非误断言数据库零写入。
    assert_eq!(
        fixture.enrollment_state().unwrap_err(),
        SessionError::MigrationRequired
    );
}

#[test]
fn owner_and_ttl_must_match_the_existing_auth_store() {
    for owner_mismatch in [true, false] {
        let fixture = Fixture::new();
        fixture.initialize(4, true, false);
        let mut value = fixture.value();
        if owner_mismatch {
            value["owner"]["subject"] = json!("another-private-subject");
        } else {
            value["idle_seconds"] = json!(601);
        }
        fixture.write_config(&value);
        fixture.reject(fixture.args());
    }
}

#[test]
fn another_valid_audit_is_rejected_without_copying_original_receipts() {
    let fixture = Fixture::new();
    fixture.initialize(4, true, false);
    let wrong = fixture.root.join("wrong-audit");
    fs::create_dir(&wrong).unwrap();
    super::fixture::mode(&wrong, 0o700);
    drop(SqliteAuditLedger::open(&wrong).unwrap());
    let mut value = fixture.value();
    value["audit_directory"] = json!(wrong);
    fixture.write_config(&value);
    fixture.reject(fixture.args());
    assert!(
        SqliteAuditLedger::open_existing(&wrong)
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        SqliteAuditLedger::open_existing(&fixture.audit)
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn new_mode_rejects_mixed_old_access_options() {
    for (flag, value) in [
        ("--access-team", ISSUER),
        ("--access-audience", AUDIENCE),
        ("--access-email", EMAIL),
    ] {
        let fixture = Fixture::new();
        let mut args = fixture.args();
        args.extend([flag.into(), value.into()]);
        fixture.reject(args);
        fixture.empty_stores();
    }
}

#[test]
fn session_config_requires_origin_and_rejects_duplicates() {
    let fixture = Fixture::new();
    let mut missing_origin = fixture.args();
    missing_origin.truncate(missing_origin.len() - 2);
    fixture.reject(missing_origin);
    for (flag, value) in [
        ("--session-config", fixture.config.to_str().unwrap()),
        ("--public-origin", ORIGIN),
    ] {
        let mut args = fixture.args();
        args.extend([flag.into(), value.into()]);
        fixture.reject(args);
    }
    fixture.empty_stores();
}

#[test]
fn config_shape_and_budgets_are_checked_before_storage() {
    for invalid in 0..6 {
        let fixture = Fixture::new();
        let mut value = fixture.value();
        match invalid {
            0 => value["version"] = json!(2),
            1 => value["unexpected"] = json!("private-unknown-canary"),
            2 => value["seed_ref"] = json!("../not-a-reference"),
            3 => value["idle_seconds"] = json!(0),
            4 => value["absolute_seconds"] = json!(604801),
            _ => {
                value.as_object_mut().unwrap().remove("owner");
            }
        }
        fixture.write_config(&value);
        fixture.reject(fixture.args());
        fixture.empty_stores();
    }
    let fixture = Fixture::new();
    fs::write(&fixture.config, vec![b' '; 16_385]).unwrap();
    fixture.reject(fixture.args());
    fixture.empty_stores();
}

#[test]
fn duplicate_json_fields_are_not_accepted() {
    let fixture = Fixture::new();
    let encoded = serde_json::to_string(&fixture.value()).unwrap();
    fs::write(
        &fixture.config,
        encoded.replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
    )
    .unwrap();
    fixture.reject(fixture.args());
    fixture.empty_stores();
}

#[test]
fn non_utf8_argument_is_rejected_without_panicking_or_echoing_bytes() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let fixture = Fixture::new();
    let mut args: Vec<OsString> = fixture.args().into_iter().map(OsString::from).collect();
    let mut invalid = b"private-non-utf8-canary-".to_vec();
    invalid.push(0xff);
    args.push(OsString::from_vec(invalid));
    super::process::reject_os(
        &fixture.root,
        args,
        &[
            "private-non-utf8-canary",
            "invalid-environment-secret-canary",
            fixture.root.to_str().unwrap(),
            EMAIL,
            AUDIENCE,
        ],
    );
    fixture.empty_stores();
}
