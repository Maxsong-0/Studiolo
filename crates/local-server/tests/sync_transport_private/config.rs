//! 合成配置契约；不读取环境/凭证，不连接账号或远端。

use super::*;
use serde_json::{Value, json};

const APPROVED: [FileType; 2] = [FileType::Pdf, FileType::Image];

fn expected() -> Expected<'static> {
    Expected {
        remote_vault_id: "synthetic-remote-id",
        mirror_path: "/synthetic/not-existing/vault",
        device_name: "Studiolo 合成",
        config_dir: ".obsidian",
        file_types: &APPROVED,
    }
}

fn value() -> Value {
    json!({
        "vaultId": "synthetic-remote-id", "vaultName": "合成远端名称",
        "vaultPath": "/synthetic/not-existing/vault", "syncMode": "bidirectional",
        "conflictStrategy": "conflict", "deviceName": "Studiolo 合成",
        "configDir": ".obsidian", "fileTypes": ["pdf", "image"],
        "configs": [], "excludedFolders": []
    })
}

fn check(value: &Value) -> Result<LocalConfigMatch, ConfigError> {
    verify(&serde_json::to_vec(value).unwrap(), &expected())
}

#[test]
fn offline_json_only_proves_local_config_match() {
    // 没有账号、进程、文件或网络输入也能成功；不可解释为任何送达证明。
    assert_eq!(check(&value()), Ok(LocalConfigMatch));
    let mut renamed = value();
    renamed["vaultName"] = json!("易变显示名不绑定身份");
    renamed["fileTypes"] = json!(["image", "pdf"]);
    assert_eq!(check(&renamed), Ok(LocalConfigMatch));
    for field in ["token", "authenticated", "synced", "receipt"] {
        let mut supplied = value();
        supplied[field] = json!("synthetic-secret-do-not-echo");
        let error = check(&supplied).unwrap_err();
        assert_eq!(error, ConfigError::Malformed);
        assert!(!error.to_string().contains("synthetic-secret"));
        assert!(!format!("{error:?}").contains("synthetic-secret"));
    }
}

#[test]
fn empty_attachments_are_an_explicit_approval_not_default_types() {
    let mut no_attachments = value();
    no_attachments["fileTypes"] = json!([]);
    let mut approval = expected();
    approval.file_types = &[];
    assert_eq!(
        verify(&serde_json::to_vec(&no_attachments).unwrap(), &approval),
        Ok(LocalConfigMatch)
    );
    assert_eq!(check(&no_attachments), Err(ConfigError::Mismatch));
    assert_eq!(
        verify(&serde_json::to_vec(&value()).unwrap(), &approval),
        Err(ConfigError::Mismatch)
    );
}

#[test]
fn every_field_is_required_and_wrong_types_are_rejected() {
    let fields = value()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 10);
    for field in fields {
        let mut missing = value();
        missing.as_object_mut().unwrap().remove(&field);
        assert_eq!(
            check(&missing),
            Err(ConfigError::Malformed),
            "missing {field}"
        );
        let mut wrong = value();
        wrong[&field] = Value::Null;
        assert_eq!(check(&wrong), Err(ConfigError::Malformed), "null {field}");
    }
    for root in [json!([]), json!(true), json!(1), json!("config")] {
        assert_eq!(check(&root), Err(ConfigError::Malformed));
    }
    let positional = json!([
        "synthetic-remote-id",
        "合成远端名称",
        "/synthetic/not-existing/vault",
        "bidirectional",
        "conflict",
        "Studiolo 合成",
        ".obsidian",
        ["pdf", "image"],
        [],
        []
    ]);
    assert_eq!(check(&positional), Err(ConfigError::Malformed));
}

#[test]
fn duplicate_escaped_unknown_and_incomplete_json_never_match() {
    let encoded = serde_json::to_string(&value()).unwrap();
    let samples = [
        format!("{{\"vaultId\":\"synthetic-remote-id\",{}", &encoded[1..]),
        format!(
            "{{\"vault\\u0049d\":\"synthetic-remote-id\",{}",
            &encoded[1..]
        ),
        format!("{{\"extra\":false,{}", &encoded[1..]),
        format!("{encoded} {{}}"),
        encoded[..encoded.len() - 1].to_owned(),
        encoded.replace("合成远端名称", "\\ud800"),
    ];
    for sample in samples {
        assert_eq!(
            verify(sample.as_bytes(), &expected()),
            Err(ConfigError::Malformed)
        );
    }
    let mut invalid_utf8 = encoded.into_bytes();
    let index = invalid_utf8.iter().position(|byte| *byte >= 128).unwrap();
    invalid_utf8[index] = 255;
    assert_eq!(
        verify(&invalid_utf8, &expected()),
        Err(ConfigError::Malformed)
    );
}

#[path = "config_validation.rs"]
mod validation;
