//! 产品参数与既有写者预检的真实进程负测，不借助网络或真实账户。

use super::fixture::{Fixture, mode};
use std::fs;
use studiolo_vault_broker::FsVaultBroker;

#[test]
fn invalid_writer_roots_fail_before_creating_auth_or_audit_stores() {
    for case in ["missing", "private-overlap", "config", "permissions"] {
        let f = Fixture::new();
        let data = match case {
            "private-overlap" => f.auth.clone(),
            "config" => f.config.parent().unwrap().to_path_buf(),
            _ => f.root.join("writer-data"),
        };
        if case == "permissions" {
            fs::create_dir(&data).unwrap();
            mode(&data, 0o755);
        }
        let mut args = f.args();
        args.extend([
            "--canonical-writer-data".into(),
            data.to_str().unwrap().into(),
        ]);
        f.reject(args);
        f.empty_stores();
        if case == "missing" {
            assert!(!data.exists());
        }
    }
}

#[test]
fn real_writer_contention_does_not_start_a_listener_or_change_binding() {
    let f = Fixture::new();
    f.initialize(3, true, false);
    let data = f.root.join("writer-data");
    fs::create_dir(&data).unwrap();
    mode(&data, 0o700);
    let owner = FsVaultBroker::open(&f.vault, &data).unwrap();
    let binding = fs::read(data.join("broker-binding.json")).unwrap();
    let mut args = f.args();
    args.extend([
        "--canonical-writer-data".into(),
        data.to_str().unwrap().into(),
    ]);
    f.reject(args);
    assert_eq!(fs::read(data.join("broker-binding.json")).unwrap(), binding);
    drop(owner);
}

#[test]
fn old_and_new_canonical_flags_cannot_be_combined() {
    let f = Fixture::new();
    let mut args = f.args();
    args.extend([
        "--canonical".into(),
        "--canonical-writer-data".into(),
        f.root.join("missing").to_str().unwrap().into(),
    ]);
    f.reject(args);
    f.empty_stores();
    assert!(!f.root.join("missing").exists());
}
