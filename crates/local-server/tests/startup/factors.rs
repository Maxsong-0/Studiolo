//! 真实产品进程的因子装配负测；必须在秘密预检失败时退出，不能进入真实取钥。

use super::{
    fixture::{Fixture, mode},
    process::Running,
};
use std::fs;

const CURRENT: &str = "STUDIOLO_TOTP_SEED_HEX";
const REPLACEMENT: &str = "STUDIOLO_TOTP_REPLACEMENT_SEED_HEX";
const VALID: &str = "3132333435363738393031323334353637383930";

fn args(f: &Fixture) -> Vec<String> {
    fs::create_dir(f.root.join("proxy")).unwrap();
    mode(&f.root.join("proxy"), 0o700);
    let mut args = f.args();
    args.extend([
        "--factor-socket".into(),
        f.root.join("proxy/http.sock").to_str().unwrap().into(),
    ]);
    args
}

fn reject(f: &Fixture, args: Vec<String>, environment: &[(&str, &str)], expected: &str) {
    let output = Running::start_factor(&f.root, args, environment).wait();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty() && output.stderr.len() <= 4096);
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains(expected), "错误阶段不符：{error}");
    for secret in ["SECRET_CANARY", VALID, f.root.to_str().unwrap()] {
        assert!(!error.contains(secret));
    }
    assert!(!error.contains("panicked"));
    assert!(!f.root.join("proxy/http.sock").exists());
}

#[test]
fn real_factor_process_rejects_missing_and_noncanonical_environment_before_storage() {
    for value in [
        None,
        Some("SECRET_CANARY"),
        Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
    ] {
        let f = Fixture::new();
        let args = args(&f);
        let env: Vec<_> = value.into_iter().map(|value| (CURRENT, value)).collect();
        reject(&f, args, &env, "因子秘密环境缺失或不规范");
        f.empty_stores();
    }
}

#[test]
fn real_factor_process_requires_paired_candidate_reference_and_material() {
    for reference_only in [false, true] {
        let f = Fixture::new();
        let mut args = args(&f);
        let mut env = vec![(CURRENT, VALID)];
        if reference_only {
            args.extend(["--replacement-seed-ref".into(), "next-reference".into()]);
        } else {
            env.push((REPLACEMENT, VALID));
        }
        reject(&f, args, &env, "因子秘密环境缺失或不规范");
        f.empty_stores();
    }
}

#[test]
fn real_factor_process_does_not_clear_existing_socket_or_mix_tcp() {
    let f = Fixture::new();
    let mut values = args(&f);
    values.extend(["--port".into(), "4317".into()]);
    reject(&f, values, &[], "不得指定TCP端口");
    f.empty_stores();

    let f = Fixture::new();
    let values = args(&f);
    let path = f.root.join("proxy/http.sock");
    fs::write(&path, b"retained-owner-file").unwrap();
    let output = Running::start_factor(&f.root, values, &[(CURRENT, "SECRET_CANARY")]).wait();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("可信Unix入口")
    );
    assert_eq!(fs::read(path).unwrap(), b"retained-owner-file");
    f.empty_stores();
}
