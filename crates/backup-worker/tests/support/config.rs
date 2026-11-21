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
/// 置备脚本（`deploy/backup-worker/provision/prereq.py`）生成的配置，原样交给真实解析。
///
/// 脚本里那套取值规则是照抄的：抄漏一条，要到 worker 启动失败才知道。夹具和脚本输出逐字节相同，
/// 由 `deploy/backup-worker/provision/test_provision.py` 钉住（账号和读取组的号码用占位值）。
#[test]
fn the_config_the_provisioning_script_generates_is_accepted_byte_for_byte() {
    let raw =
        include_bytes!("../../../../deploy/backup-worker/provision/fixtures/backup-worker.json");
    let config = WorkerConfig::parse(raw, 64001).unwrap();
    assert_eq!(config.producer_uid(), 1000);
    assert_eq!(config.reader_gid(), 64002);
    assert_eq!(
        config.sealed_parent(),
        Path::new("/var/lib/studiolo-backup-staging")
    );
    assert_eq!(
        config.producer_socket(),
        Path::new("/run/studiolo-backup/producer.sock")
    );
    assert_eq!(
        config.repository().as_str(),
        "rest:https://192.168.100.111:18443/studiolo-backup-worker/main/"
    );
    assert_eq!(config.repository_ca_sha256().map(str::len), Some(64));
    // 号码是按账号核的：换一个账号来读就拒绝。
    assert!(WorkerConfig::parse(raw, 64003).is_err());
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

/// 一份虚构的 PEM 证书外形；只核外形和摘要，不解析证书本身（那是 restic 的事）。
const FICTIONAL_CA: &[u8] =
    b"-----BEGIN CERTIFICATE-----\nZmljdGlvbmFsIGNh\n-----END CERTIFICATE-----\n";

#[test]
fn a_repository_ca_is_optional_and_only_its_digest_lives_in_the_config() {
    let raw = serde_json::to_vec(&document()).unwrap();
    let config = WorkerConfig::parse(&raw, 1001).unwrap();
    assert_eq!(config.repository_ca_sha256(), None);
    assert_eq!(config.repository_ca(), None);

    let digest = hex(&Sha256::digest(FICTIONAL_CA));
    let mut doc = document();
    doc["repository_ca_sha256"] = serde_json::json!(digest);
    let config = WorkerConfig::parse(&serde_json::to_vec(&doc).unwrap(), 1001).unwrap();
    assert_eq!(config.repository_ca_sha256(), Some(digest.as_str()));
    // 文件只在 load 里读过、核过摘要之后才交给 restic；只解析配置不算数。
    assert_eq!(config.repository_ca(), None);

    for bad in [
        serde_json::json!("AB".repeat(32)),
        serde_json::json!("ab".repeat(31)),
        serde_json::json!(""),
        serde_json::json!(null),
        serde_json::json!(1),
    ] {
        let mut doc = document();
        doc["repository_ca_sha256"] = bad.clone();
        // `null` 等于没给（serde 的 Option），其余都拒绝。
        let parsed = WorkerConfig::parse(&serde_json::to_vec(&doc).unwrap(), 1001);
        assert_eq!(parsed.is_ok(), bad.is_null(), "{bad}");
    }
}

#[test]
fn the_ca_file_must_match_the_approved_digest_and_look_like_a_certificate() {
    let digest = hex(&Sha256::digest(FICTIONAL_CA));
    assert!(verify_ca(FICTIONAL_CA, &digest).is_ok());
    // 换了一个字节：摘要对不上就拒绝，不退回系统信任库。
    let mut tampered = FICTIONAL_CA.to_vec();
    tampered[30] ^= 1;
    assert!(verify_ca(&tampered, &digest).is_err());
    for wrong in [
        &b"not a certificate"[..],
        &b"-----BEGIN CERTIFICATE-----\n"[..],
        &b"-----BEGIN PRIVATE KEY-----\n-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n"[..],
        &[0xff, 0xfe][..],
    ] {
        assert!(verify_ca(wrong, &hex(&Sha256::digest(wrong))).is_err());
    }
}
