//! 非秘密配置的公开加载契约；全部身份与路径均为虚构临时夹具。

#[path = "support/config_path_tests.rs"]
mod paths;

#[path = "support/serving_tests.rs"]
mod serving;

#[path = "support/canonical_tests.rs"]
mod canonical;

#[path = "support/access_tests.rs"]
mod access;

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use studiolo_host_config::Config;

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    path: PathBuf,
    value: Value,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for name in ["config", "auth", "audit"] {
            private_dir(&root.join(name));
        }
        let value = json!({"version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture-audience",
            "subject":"fictional-person","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"test-only-owner"});
        let fixture = Self {
            _directory: directory,
            path: root.join("config/setup.json"),
            root,
            value,
        };
        fixture.write(&fixture.value);
        fixture
    }

    fn bytes(&self, bytes: &[u8]) {
        fs::write(&self.path, bytes).unwrap();
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn write(&self, value: &Value) {
        self.bytes(&serde_json::to_vec(value).unwrap());
    }

    fn rejects(&self, value: &Value) {
        self.write(value);
        assert!(Config::load(&self.path).is_err());
    }
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn valid_config_loads_without_initializing_either_database() {
    let f = Fixture::new();
    let loaded = Config::load(&f.path).unwrap();
    let sessions = serde_json::to_value(loaded.sessions().unwrap()).unwrap();
    assert_eq!(sessions["owner"]["subject"], "fictional-person");
    assert_eq!(loaded.seed_ref(), "test-only-owner");
    assert!(loaded.sessions().is_ok());
    for path in [loaded.auth_directory(), loaded.audit_directory()] {
        assert_eq!(fs::read_dir(path).unwrap().count(), 0);
    }
}

#[test]
fn unknown_missing_duplicate_and_wrongly_typed_fields_are_rejected() {
    let f = Fixture::new();
    for key in f.value.as_object().unwrap().keys() {
        let mut value = f.value.clone();
        value.as_object_mut().unwrap().remove(key);
        f.rejects(&value);
    }
    for key in ["issuer", "audience", "subject", "email"] {
        let mut value = f.value.clone();
        value["owner"].as_object_mut().unwrap().remove(key);
        f.rejects(&value);
    }
    for (pointer, value) in [
        ("/unknown", json!(true)),
        ("/owner/unknown", json!(true)),
        ("/version", json!(2)),
        ("/version", json!(256)),
        ("/version", json!("1")),
        ("/idle_seconds", json!(-1)),
        ("/absolute_seconds", json!(1.5)),
        ("/seed_ref", Value::Null),
        (
            "/owner",
            json!([
                "https://fixture.cloudflareaccess.com",
                "fixture-audience",
                "fictional-person",
                "owner@example.invalid"
            ]),
        ),
    ] {
        let mut config = f.value.clone();
        if pointer == "/unknown" {
            config["unknown"] = value;
        } else if pointer == "/owner/unknown" {
            config["owner"]["unknown"] = value;
        } else {
            *config.pointer_mut(pointer).unwrap() = value;
        }
        f.rejects(&config);
    }
    let encoded = serde_json::to_string(&f.value).unwrap();
    for duplicate in [
        format!("{{\"version\":1,{}", &encoded[1..]),
        encoded.replacen(
            "\"owner\":{",
            "\"owner\":{\"subject\":\"fictional-person\",",
            1,
        ),
    ] {
        f.bytes(duplicate.as_bytes());
        assert!(Config::load(&f.path).is_err());
    }
}

#[test]
fn owner_constraints_are_the_existing_auth_binding_constraints() {
    let f = Fixture::new();
    for (key, maximum) in [
        ("issuer", 512),
        ("audience", 256),
        ("subject", 256),
        ("email", 254),
    ] {
        for value in [
            String::new(),
            " \t".into(),
            "bad\nvalue".into(),
            "x".repeat(maximum + 1),
        ] {
            let mut config = f.value.clone();
            config["owner"][key] = json!(value);
            f.rejects(&config);
        }
    }
    let mut config = f.value.clone();
    config["owner"]["issuer"] = json!("http://fixture.example.invalid");
    f.rejects(&config);
}

#[test]
fn explicit_ttl_edges_do_not_silently_default_or_clamp() {
    let f = Fixture::new();
    for (idle, absolute) in [(0, 1), (2, 1), (1, 0), (1, 604_801)] {
        let mut config = f.value.clone();
        config["idle_seconds"] = json!(idle);
        config["absolute_seconds"] = json!(absolute);
        f.rejects(&config);
    }
    let mut config = f.value.clone();
    config["idle_seconds"] = json!(604_800);
    config["absolute_seconds"] = json!(604_800);
    f.write(&config);
    assert!(Config::load(&f.path).is_ok());
}

#[test]
fn seed_reference_is_bounded_ascii_data_not_a_file_path() {
    let f = Fixture::new();
    for reference in [
        "".into(),
        "x".repeat(65),
        "a/b".into(),
        "a\\b".into(),
        "a b".into(),
        "种子".into(),
        "a\nb".into(),
    ] {
        let mut config = f.value.clone();
        config["seed_ref"] = json!(reference);
        f.rejects(&config);
    }
    for reference in ["._-a1".into(), "x".repeat(64)] {
        let mut config = f.value.clone();
        config["seed_ref"] = json!(reference);
        f.write(&config);
        assert!(Config::load(&f.path).is_ok());
    }
}

#[test]
fn exactly_sixteen_kib_is_allowed_but_larger_empty_or_non_utf8_is_rejected() {
    let f = Fixture::new();
    let mut bytes = serde_json::to_vec(&f.value).unwrap();
    bytes.resize(16_384, b' ');
    f.bytes(&bytes);
    assert!(Config::load(&f.path).is_ok());
    bytes.push(b' ');
    for invalid in [bytes, Vec::new(), vec![0xff]] {
        f.bytes(&invalid);
        assert!(Config::load(&f.path).is_err());
        assert_eq!(fs::read(&f.path).unwrap(), invalid);
    }
}
