//! 备份审批的行为与失败契约：默认关闭、固定包、预算满足 RPO、
//! 普通目录不得声明不可变保护、凭据只写变量名。

use super::*;

fn approval() -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "target_shape": "dedicated_append_only_target",
        "claims_immutability": true,
        "restic_package": "restic",
        "restic_version": "0.18.2",
        "schedule_interval_millis": 600_000,
        "completion_budget_millis": 300_000,
        "rpo_millis": 900_000,
        "keep_last": 30,
        "keep_daily": 14,
        "credentials": [
            { "logical_name": "restic_repository_password",
              "environment_variable": "RESTIC_PASSWORD_FILE" }
        ]
    })
}

fn parse(value: serde_json::Value) -> Result<Option<BackupApproval>, serde_json::Error> {
    decode(value)
}

#[test]
fn an_approved_dedicated_target_parses_with_its_fixed_package_and_budgets() {
    let approved = parse(approval()).expect("decode").expect("approval");

    assert_eq!(
        approved.target_shape(),
        BackupTargetShape::DedicatedAppendOnlyTarget
    );
    assert!(approved.claims_immutability());
    assert_eq!(approved.restic().package(), "restic");
    assert_eq!(approved.restic().version(), "0.18.2");
    assert!(approved.schedule().satisfies_rpo());
    assert_eq!(approved.retention().keep_last(), 30);
    assert_eq!(
        approved.credentials()[0].environment_variable(),
        "RESTIC_PASSWORD_FILE"
    );
}

#[test]
fn a_plain_smb_or_nfs_directory_may_not_claim_immutable_protection() {
    let mut value = approval();
    value["target_shape"] = serde_json::json!("plain_smb_or_nfs_directory");
    assert!(parse(value.clone()).is_err(), "声明造不出不可变保护");

    value["claims_immutability"] = serde_json::json!(false);
    let approved = parse(value).expect("decode").expect("approval");
    assert!(!approved.claims_immutability());
    assert!(!approved.target_shape().may_claim_immutability());
}

#[test]
fn a_local_temporary_repository_may_not_claim_immutable_protection_either() {
    let mut value = approval();
    value["target_shape"] = serde_json::json!("local_temporary_repository");
    assert!(parse(value).is_err());
}

#[test]
fn a_budget_that_cannot_meet_the_declared_rpo_is_refused() {
    let mut value = approval();
    value["completion_budget_millis"] = serde_json::json!(300_001);
    assert!(parse(value).is_err());
}

#[test]
fn only_the_fixed_restic_package_and_a_numeric_version_are_accepted() {
    for (key, bad) in [
        ("restic_package", serde_json::json!("borg")),
        ("restic_version", serde_json::json!("latest")),
        ("restic_version", serde_json::json!("")),
    ] {
        let mut value = approval();
        value[key] = bad.clone();
        assert!(parse(value).is_err(), "{key}={bad}");
    }
}

#[test]
fn a_credential_entry_that_looks_like_a_value_rather_than_a_variable_name_is_refused() {
    for bad in [
        "RESTIC_PASSWORD=hunter2",
        "restic_password",
        "RESTIC PASSWORD",
        "1PASSWORD",
        "",
    ] {
        let mut value = approval();
        value["credentials"][0]["environment_variable"] = serde_json::json!(bad);
        assert!(parse(value).is_err(), "{bad}");
    }
}

#[test]
fn an_unknown_field_or_version_fails_closed() {
    let mut extra = approval();
    extra["surprise"] = serde_json::json!(true);
    assert!(parse(extra).is_err());

    let mut version = approval();
    version["version"] = serde_json::json!(2);
    assert!(parse(version).is_err());
}

#[test]
fn an_empty_retention_or_credential_list_is_refused() {
    let mut retention = approval();
    retention["keep_daily"] = serde_json::json!(0);
    assert!(parse(retention).is_err());

    let mut credentials = approval();
    credentials["credentials"] = serde_json::json!([]);
    assert!(parse(credentials).is_err());
}
