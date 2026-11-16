use super::*;
fn document() -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "worker_uid": 1001,
        "producer_uid": 1002,
        "reader_gid": 1003,
        "sealed_parent": "/var/lib/studiolo-checkpoints",
        "producer_socket": "/run/studiolo-human/backup-audit.sock",
        "repository": "rest:https://backup.example.test/repository/",
        "restic_sha256": "ab".repeat(32),
    })
}
#[test]
fn fixed_schema_binds_independent_identities_repository_and_original_bytes() {
    let raw = serde_json::to_vec(&document()).unwrap();
    let config = WorkerConfig::parse(&raw, 1001).unwrap();
    assert_eq!(config.worker_uid(), 1001);
    assert_eq!(config.producer_uid(), 1002);
    assert_eq!(config.reader_gid(), 1003);
    assert_eq!(
        config.sealed_parent(),
        Path::new("/var/lib/studiolo-checkpoints")
    );
    assert_eq!(
        config.producer_socket(),
        Path::new("/run/studiolo-human/backup-audit.sock")
    );
    assert_eq!(
        config.repository().as_str(),
        "rest:https://backup.example.test/repository/"
    );
    assert_eq!(config.restic_sha256(), "ab".repeat(32));
    let pretty =
        WorkerConfig::parse(&serde_json::to_vec_pretty(&document()).unwrap(), 1001).unwrap();
    assert_ne!(config.digest(), pretty.digest());
    assert_eq!(config.digest().len(), 64);
}
#[test]
fn config_rejects_identity_confusion_secrets_and_unbounded_or_unsafe_locations() {
    for (key, value) in [
        ("schema", serde_json::json!(2)),
        ("worker_uid", serde_json::json!(1002)),
        ("producer_uid", serde_json::json!(1001)),
        ("producer_uid", serde_json::json!(0)),
        ("reader_gid", serde_json::json!(0)),
        ("restic_sha256", serde_json::json!("latest")),
        ("restic_sha256", serde_json::json!("AB".repeat(32))),
        ("sealed_parent", serde_json::json!("/")),
        ("sealed_parent", serde_json::json!("/var/lib")),
        (
            "sealed_parent",
            serde_json::json!("/tmp/../var/lib/staging"),
        ),
        ("sealed_parent", serde_json::json!("/tmp/./staging")),
        ("sealed_parent", serde_json::json!("/tmp//staging")),
        ("sealed_parent", serde_json::json!("/tmp/staging/")),
        ("sealed_parent", serde_json::json!("/tmp/staging;run")),
        (
            "producer_socket",
            serde_json::json!("/var/lib/studiolo-checkpoints/a.sock"),
        ),
        (
            "producer_socket",
            serde_json::json!(format!("/run/{}", "a".repeat(101))),
        ),
        ("repository", serde_json::json!("sftp:host:/repo")),
        (
            "repository_password",
            serde_json::json!("fictional-secret-for-rejection"),
        ),
    ] {
        let mut doc = document();
        doc[key] = value;
        assert!(
            WorkerConfig::parse(&serde_json::to_vec(&doc).unwrap(), 1001).is_err(),
            "{key}"
        );
    }
    for uid in [0, 1002, u32::MAX] {
        assert!(WorkerConfig::parse(&serde_json::to_vec(&document()).unwrap(), uid).is_err());
    }
    let mut missing = document();
    missing.as_object_mut().unwrap().remove("producer_uid");
    assert!(WorkerConfig::parse(&serde_json::to_vec(&missing).unwrap(), 1001).is_err());
    assert!(WorkerConfig::parse(&[b' '; 16 * 1024 + 1], 1001).is_err());
    let duplicate =
        serde_json::to_string(&document())
            .unwrap()
            .replacen("{", "{\"worker_uid\":1001,", 1);
    assert!(WorkerConfig::parse(duplicate.as_bytes(), 1001).is_err());
}
