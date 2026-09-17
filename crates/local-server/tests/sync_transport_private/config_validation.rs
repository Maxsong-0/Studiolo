//! 绑定、批准范围与容量的纯数据负测。

use super::*;

#[test]
fn identity_device_config_and_modes_are_exact() {
    for (field, replacement) in [
        ("vaultId", "another-remote"),
        ("deviceName", "another-device"),
        ("configDir", ".another"),
        ("syncMode", "pull-only"),
        ("syncMode", "mirror-remote"),
        ("conflictStrategy", "merge"),
        ("vaultPath", "/synthetic/not-existing/Vault"),
    ] {
        let mut changed = value();
        changed[field] = json!(replacement);
        assert_eq!(check(&changed), Err(ConfigError::Mismatch), "{field}");
    }
    for field in ["syncMode", "conflictStrategy"] {
        let mut changed = value();
        changed[field] = json!(["conflict"]);
        assert_eq!(check(&changed), Err(ConfigError::Malformed));
        changed[field] = json!({"bidirectional": null});
        assert_eq!(check(&changed), Err(ConfigError::Malformed));
        changed[field] = json!("unknown");
        assert_eq!(check(&changed), Err(ConfigError::Mismatch));
    }
}

#[test]
fn paths_are_not_canonicalized_case_folded_or_unicode_normalized() {
    for path in [
        "relative/vault",
        "/",
        "/synthetic//vault",
        "/synthetic/./vault",
        "/synthetic/../vault",
        "/synthetic/vault/",
        "/synthetic\\vault",
    ] {
        let mut changed = value();
        changed["vaultPath"] = json!(path);
        assert_eq!(check(&changed), Err(ConfigError::InvalidValue), "{path}");
    }
    let mut approval = expected();
    approval.mirror_path = "/synthetic/café";
    let mut changed = value();
    changed["vaultPath"] = json!("/synthetic/cafe\u{301}");
    assert_eq!(
        verify(&serde_json::to_vec(&changed).unwrap(), &approval),
        Err(ConfigError::Mismatch)
    );
}

#[test]
fn arrays_are_closed_bounded_and_cannot_enable_filters() {
    for field in ["configs", "excludedFolders"] {
        for item in [json!("one"), json!(null), json!({"nested": [1, 2]})] {
            let mut changed = value();
            changed[field] = json!([item]);
            assert_eq!(check(&changed), Err(ConfigError::Malformed));
        }
    }
    for types in [
        json!(["pdf", "unknown"]),
        json!(["pdf", false]),
        json!([{"pdf": null}, "image"]),
        json!(["image", "audio", "video", "pdf", "unsupported", "pdf"]),
    ] {
        let mut changed = value();
        changed["fileTypes"] = types;
        assert_eq!(check(&changed), Err(ConfigError::Malformed));
    }
    let mut changed = value();
    changed["fileTypes"] = json!(["pdf", "pdf"]);
    assert_eq!(check(&changed), Err(ConfigError::InvalidValue));
    changed["fileTypes"] = json!(["pdf", "audio"]);
    assert_eq!(check(&changed), Err(ConfigError::Mismatch));
}

#[test]
fn input_and_strings_use_byte_budgets() {
    let mut encoded = serde_json::to_vec(&value()).unwrap();
    encoded.resize(INPUT_BYTES, b' ');
    assert_eq!(verify(&encoded, &expected()), Ok(LocalConfigMatch));
    encoded.push(b' ');
    assert_eq!(
        verify(&encoded, &expected()),
        Err(ConfigError::InputTooLarge)
    );
    let mut changed = value();
    changed["vaultName"] = json!("界".repeat(85));
    assert_eq!(check(&changed), Ok(LocalConfigMatch));
    changed["vaultName"] = json!("界".repeat(86));
    assert_eq!(check(&changed), Err(ConfigError::InvalidValue));
    for (field, overlong) in [
        ("vaultId", "x".repeat(TEXT_BYTES + 1)),
        ("deviceName", "x".repeat(TEXT_BYTES + 1)),
        ("configDir", format!(".{}", "x".repeat(CONFIG_DIR_BYTES))),
        ("vaultPath", format!("/{}", "x".repeat(PATH_BYTES))),
    ] {
        let mut changed = value();
        changed[field] = json!(overlong);
        assert_eq!(check(&changed), Err(ConfigError::InvalidValue));
    }
    for text in ["", " \t", "valid\u{0}", "valid\u{85}"] {
        let mut changed = value();
        changed["vaultName"] = json!(text);
        assert_eq!(check(&changed), Err(ConfigError::InvalidValue));
    }
}

#[test]
fn invalid_expected_parameters_cannot_authorize_matching_bad_data() {
    let bytes = serde_json::to_vec(&value()).unwrap();
    let mut approval = expected();
    approval.mirror_path = "/synthetic/../vault";
    assert_eq!(verify(&bytes, &approval), Err(ConfigError::InvalidExpected));
    let mut approval = expected();
    approval.file_types = &[FileType::Pdf, FileType::Pdf];
    assert_eq!(verify(&bytes, &approval), Err(ConfigError::InvalidExpected));
    let mut approval = expected();
    approval.config_dir = ".obsidian/../other";
    assert_eq!(verify(&bytes, &approval), Err(ConfigError::InvalidExpected));
    let mut approval = expected();
    approval.remote_vault_id = "";
    assert_eq!(verify(&bytes, &approval), Err(ConfigError::InvalidExpected));
}
