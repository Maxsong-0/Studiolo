use super::super::super::prepare_composed;
use super::super::{Fixture, StartupError, prepare_with, test_support, writer::writer_options};
use super::support::{PROJECT, configure, cookie, generation, issue, json, proxy, request, uri};
use std::{fs, os::unix::fs::PermissionsExt};

#[tokio::test]
async fn overlap_is_rejected_before_database_open_secret_slots_and_external_keys() {
    for root in ["auth", "vault", "config", "proxy"] {
        let f = Fixture::in_tmp(true);
        configure(&f, true);
        let mut options = f.options();
        options.factor = Some(crate::options::FactorOptions {
            socket: proxy(&f),
            replacement_seed_ref: None,
        });
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
        value["project_repository"]["directory"] = serde_json::json!(f.root.join(root));
        fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
        // 双库也故意不完整；必须先报根审批错误，不能进入数据库或秘密读取。
        fs::remove_file(f.root.join("auth/auth.sqlite3")).unwrap();
        let result = prepare_composed(
            options,
            |_| async { panic!("审批失败不得取钥") },
            || panic!("审批失败不得读取环境秘密"),
        )
        .await;
        assert!(matches!(result, Err(StartupError::Configuration)), "{root}");
        assert!(!f.root.join("proxy/http.sock").exists());
        f.unbound();
    }
}

#[tokio::test]
async fn writer_data_overlap_is_rejected_before_attempting_the_writer_lease() {
    let f = Fixture::in_tmp(true);
    configure(&f, true);
    let (options, data) = writer_options(&f);
    let held =
        studiolo_vault_broker::FsVaultBroker::open_existing(&f.root.join("vault"), &data).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    value["project_repository"]["directory"] = serde_json::json!(data);
    fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        prepare_with(options, |_| async { panic!("无效根不得取钥") }).await,
        Err(StartupError::Configuration)
    ));
    drop(held);
    f.unbound();
}

#[tokio::test]
async fn configured_rust_source_list_is_rejected_before_inputs() {
    let f = Fixture::in_tmp(true);
    configure(&f, false);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    value["project_repository"]["rust_call_sites"] = serde_json::json!(["src/lib.rs"]);
    fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
    let mut options = f.options();
    options.vault = f.root.join("must-not-open");
    assert!(matches!(
        prepare_with(options, |_| async {
            panic!("生产源码未获隔离不得取钥")
        })
        .await,
        Err(StartupError::Configuration)
    ));
    assert!(!f.root.join("must-not-open").exists());
}

#[tokio::test]
async fn repository_and_protected_root_replacement_after_prepare_prevent_listen() {
    for root in ["repository", "auth", "web", "config"] {
        let f = Fixture::in_tmp(true);
        configure(&f, true);
        let prepared = prepare_with(f.options(), |_| async {
            Ok(test_support::verifier().await)
        })
        .await
        .unwrap();
        fs::rename(f.root.join(root), f.root.join(format!("retained-{root}"))).unwrap();
        fs::create_dir(f.root.join(root)).unwrap();
        fs::set_permissions(f.root.join(root), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            matches!(prepared.bind().await, Err(StartupError::ReadRoots)),
            "{root}"
        );
        f.unbound();
    }
}

#[tokio::test]
async fn factor_mode_uses_only_real_known_roots_and_keeps_default_cargo_disabled() {
    let f = Fixture::in_tmp(true);
    let repository = configure(&f, false);
    let mut options = f.options();
    options.factor = Some(crate::options::FactorOptions {
        socket: proxy(&f),
        replacement_seed_ref: None,
    });
    let prepared = prepare_composed(
        options,
        |_| async { Ok(test_support::verifier().await) },
        || {
            assert!(repository.is_dir());
            // 这是原有内存环境槽，不伪称它来自某个未经证明的文件目录。
            (
                Some("3132333435363738393031323334353637383930".into()),
                None,
            )
        },
    )
    .await
    .unwrap();
    // Unix 模式使用公共 Host；本测试只调用会话及只读接口，不伪造因子来源证明。
    let unix_request = |method, uri: &str, cookie: Option<&str>| {
        let mut request = request(&f, method, uri, cookie);
        request
            .headers_mut()
            .insert("host", "studiolo.example.com".parse().unwrap());
        request
    };
    let issued = issue(
        &prepared.app,
        unix_request("POST", "/api/v1/sessions", None),
    )
    .await;
    let current = json(
        &prepared.app,
        unix_request("GET", "/api/workbench", Some(&issued)),
        200,
    )
    .await["generation"]
        .as_u64()
        .unwrap();
    let result = json(
        &prepared.app,
        unix_request("GET", &uri(PROJECT, current), Some(&issued)),
        200,
    )
    .await;
    assert_eq!(result["cargo_dependencies"]["status"], "disabled");
    assert_eq!(result["rust_call_sites"]["status"], "disabled");
    let (listener, _) = prepared.bind_unix().unwrap();
    drop(listener);
    assert!(!f.root.join("proxy/http.sock").exists());
}

#[tokio::test]
async fn one_invalid_cargo_capture_does_not_erase_independent_directory_and_design_evidence() {
    let f = Fixture::in_tmp(true);
    let repository = configure(&f, true);
    fs::write(repository.join("Cargo.toml"), "[package\n").unwrap();
    let prepared = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    let issued = cookie(&f, &prepared.app).await;
    let current = generation(&f, &prepared.app, &issued).await;
    let result = json(
        &prepared.app,
        request(&f, "GET", &uri(PROJECT, current), Some(&issued)),
        200,
    )
    .await;
    assert_eq!(result["repository"]["status"], "ready");
    assert_eq!(result["architecture"]["status"], "ready");
    assert_eq!(result["cargo_dependencies"]["status"], "invalid");
    assert!(result["cargo_dependencies"]["graph"].is_null());
}
