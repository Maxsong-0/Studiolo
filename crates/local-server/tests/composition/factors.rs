//! 私有双槽输入Seam只用虚构材料；原生产取钥入口和真实Unix监听保持分离。

use super::super::prepare_composed;
use super::{Fixture, StartupError, test_support};
use crate::options::FactorOptions;
use std::{ffi::OsString, fs, os::unix::fs::PermissionsExt};

fn options(f: &Fixture) -> crate::options::Options {
    let proxy = f.root.join("proxy");
    fs::create_dir(&proxy).unwrap();
    fs::set_permissions(&proxy, fs::Permissions::from_mode(0o700)).unwrap();
    let mut options = f.options();
    options.factor = Some(FactorOptions {
        socket: proxy.join("http.sock"),
        replacement_seed_ref: Some("next-seed".into()),
    });
    options
}

fn environment() -> (Option<OsString>, Option<OsString>) {
    (
        Some("3132333435363738393031323334353637383930".into()),
        Some("3039383736353433323130393837363534333231".into()),
    )
}

#[tokio::test]
async fn ordinary_session_mode_never_reads_factor_material() {
    let f = Fixture::new(true);
    let prepared = prepare_composed(
        f.options(),
        |_| async { Ok(test_support::verifier().await) },
        || panic!("普通会话模式不得读取秘密槽"),
    )
    .await
    .unwrap();
    assert!(prepared.ingress.is_none());
    let (listener, _) = prepared.bind().await.unwrap();
    assert_eq!(listener.local_addr().unwrap().port(), f.port);
}

#[tokio::test]
async fn factor_preparation_is_unbound_and_cannot_bind_as_tcp() {
    let f = Fixture::new(true);
    let configured = options(&f);
    let socket = configured.factor.as_ref().unwrap().socket.clone();
    let prepared = prepare_composed(
        configured,
        |_| async { Ok(test_support::verifier().await) },
        environment,
    )
    .await
    .unwrap();
    assert!(!socket.exists());
    f.unbound();
    assert!(matches!(
        prepared.bind().await,
        Err(StartupError::Configuration)
    ));
    assert!(!socket.exists());
}

#[tokio::test]
async fn factor_unix_binding_requires_final_identity_and_removes_only_owned_socket() {
    let f = Fixture::new(true);
    let configured = options(&f);
    let socket = configured.factor.as_ref().unwrap().socket.clone();
    let prepared = prepare_composed(
        configured,
        |_| async { Ok(test_support::verifier().await) },
        environment,
    )
    .await
    .unwrap();
    let (listener, _) = prepared.bind_unix().unwrap();
    assert!(socket.exists());
    assert_eq!(
        fs::symlink_metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    f.unbound();
    drop(listener);
    assert!(!socket.exists());
}

#[tokio::test]
async fn socket_layout_is_checked_before_environment_or_external_keys() {
    let f = Fixture::new(true);
    let mut configured = options(&f);
    configured.factor.as_mut().unwrap().socket = f.root.join("vault/http.sock");
    let result = prepare_composed(
        configured,
        |_| async { panic!("不安全入口不得取钥") },
        || panic!("不安全入口不得读秘密"),
    )
    .await;
    assert!(matches!(result, Err(StartupError::Ingress)));
    f.unbound();
}

#[tokio::test]
async fn absent_or_mismatched_material_fails_before_keys_and_listen() {
    for case in [
        "missing",
        "invalid",
        "wrong-current",
        "same-reference",
        "same-seed",
    ] {
        let f = Fixture::new(true);
        let mut configured = options(&f);
        let (mut current, mut replacement) = environment();
        match case {
            "missing" => current = None,
            "invalid" => replacement = Some("SYNTHETIC_SECRET_CANARY".into()),
            "wrong-current" => current = Some("0000000000000000000000000000000000000000".into()),
            "same-reference" => {
                configured.factor.as_mut().unwrap().replacement_seed_ref =
                    Some("synthetic-seed".into())
            }
            "same-seed" => replacement = current.clone(),
            _ => unreachable!(),
        }
        let result = prepare_composed(
            configured,
            |_| async { panic!("无效因子不得取钥") },
            || (current, replacement),
        )
        .await;
        let error = result.err().unwrap();
        assert!(matches!(
            error,
            StartupError::FactorEnvironment | StartupError::Auth(_)
        ));
        assert!(!format!("{error:?} {error}").contains("SYNTHETIC_SECRET_CANARY"));
        assert!(!f.root.join("proxy/http.sock").exists());
        f.unbound();
    }
}

#[tokio::test]
async fn factor_socket_parent_change_after_prepare_prevents_listen() {
    let f = Fixture::new(true);
    let configured = options(&f);
    let prepared = prepare_composed(
        configured,
        |_| async { Ok(test_support::verifier().await) },
        environment,
    )
    .await
    .unwrap();
    fs::rename(f.root.join("proxy"), f.root.join("old-proxy")).unwrap();
    fs::create_dir(f.root.join("proxy")).unwrap();
    fs::set_permissions(f.root.join("proxy"), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(prepared.bind_unix(), Err(StartupError::Ingress)));
    assert!(!f.root.join("proxy/http.sock").exists());
    assert!(!f.root.join("old-proxy/http.sock").exists());
    f.unbound();
}
