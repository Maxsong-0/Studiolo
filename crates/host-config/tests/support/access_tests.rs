//! Access只从同一主人数据派生；测试不连接JWKS、不构造签名身份。

use super::*;

#[test]
fn access_configuration_reuses_owner_and_requires_a_canonical_public_https_origin() {
    let f = Fixture::new();
    let config = Config::load(&f.path).unwrap();
    assert!(config.access("https://studiolo.example.invalid").is_ok());
    for origin in [
        "",
        "http://studiolo.example.invalid",
        "https://studiolo.example.invalid/",
        "https://studiolo.example.invalid/path",
        "https://user@studiolo.example.invalid",
        "https://studiolo.example.invalid?x=1",
    ] {
        assert!(config.access(origin).is_err());
    }
    let sessions = serde_json::to_value(config.sessions().unwrap()).unwrap();
    assert_eq!(sessions["owner"], f.value["owner"]);
    assert_eq!(sessions["policy"]["idle_seconds"], f.value["idle_seconds"]);
    assert_eq!(
        sessions["policy"]["absolute_seconds"],
        f.value["absolute_seconds"]
    );
}

#[test]
fn serving_access_does_not_replace_invalid_owner_fields_with_separate_defaults() {
    for (key, value) in [
        ("issuer", "https://not-cloudflare.example.invalid"),
        ("audience", "space is not an Access audience"),
        ("email", "not-an-email"),
    ] {
        let f = Fixture::new();
        let mut document = f.value.clone();
        document["owner"][key] = json!(value);
        f.write(&document);
        // 原管理命令仍沿用OwnerBinding语义；只有公网装配再应用Access的严格约束。
        let config = Config::load(&f.path).unwrap();
        assert!(config.sessions().is_ok());
        assert!(config.access("https://studiolo.example.invalid").is_err());
    }
}

#[test]
fn configuration_error_never_echoes_rejected_input() {
    let f = Fixture::new();
    f.bytes(br#"{"unrecognized":"synthetic-config-sentinel"}"#);
    let error = Config::load(&f.path).err().unwrap();
    assert_eq!(error.to_string(), "非秘密配置或私有路径检查失败");
    assert!(!format!("{error:?}").contains("synthetic-config-sentinel"));
}
