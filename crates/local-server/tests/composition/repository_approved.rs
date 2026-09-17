//! 真实私有配置、双库、Access/会话与 Broker 的生产装配；只替换固定公钥来源。
#[path = "repository_approved/failures.rs"]
mod failures;
mod support;

use super::{Fixture, prepare_with, test_support};
use std::{fs, sync::Arc};
use support::{PROJECT, configure, cookie, generation, json, request, uri};

#[tokio::test]
async fn approved_repository_works_with_sessions_canonical_read_and_the_single_writer() {
    for mode in ["session", "canonical", "writer"] {
        let f = Fixture::in_tmp(true);
        let repository = configure(&f, true);
        let original = fs::read(repository.join("Cargo.toml")).unwrap();
        let mut options = f.options();
        if mode != "session" {
            let (writer, _) = super::writer::writer_options(&f);
            if mode == "writer" {
                options = writer;
            } else {
                options.canonical = true;
            }
        }
        let prepared = prepare_with(options, |_| async { Ok(test_support::verifier().await) })
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
        assert_eq!(result["cargo_dependencies"]["status"], "ready");
        assert_eq!(result["rust_call_sites"]["status"], "disabled");
        assert!(result["rust_call_sites"]["links"].is_null());
        assert!(!result.to_string().contains("SOURCE_BODY_MUST_STAY_PRIVATE"));
        assert!(!result.to_string().contains(repository.to_str().unwrap()));
        let workbench = prepared.workbench.as_ref().map(Arc::clone);
        let (listener, app) = prepared.bind().await.unwrap();
        drop((listener, app));
        if let Some(workbench) = workbench {
            workbench.stop();
            workbench.shutdown().await.unwrap();
        }
        assert_eq!(fs::read(repository.join("Cargo.toml")).unwrap(), original);
        f.unbound();
    }
}

#[tokio::test]
async fn existing_browser_gates_generation_and_project_scope_precede_repository_evidence() {
    let f = Fixture::in_tmp(true);
    configure(&f, true);
    let prepared = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    let issued = cookie(&f, &prepared.app).await;
    let current = generation(&f, &prepared.app, &issued).await;
    let path = uri(PROJECT, current);
    json(&prepared.app, request(&f, "GET", &path, None), 401).await;
    let mut absent = request(&f, "GET", &path, Some(&issued));
    absent.headers_mut().remove("cf-access-jwt-assertion");
    json(&prepared.app, absent, 401).await;
    let mut foreign = request(&f, "GET", &path, Some(&issued));
    foreign
        .headers_mut()
        .insert("origin", "https://elsewhere.invalid".parse().unwrap());
    json(&prepared.app, foreign, 403).await;
    json(
        &prepared.app,
        request(&f, "GET", &uri(PROJECT, current + 1), Some(&issued)),
        409,
    )
    .await;
    json(
        &prepared.app,
        request(&f, "GET", &uri("项目/不存在", current), Some(&issued)),
        404,
    )
    .await;
    let other = json(
        &prepared.app,
        request(&f, "GET", &uri("项目/其他", current), Some(&issued)),
        200,
    )
    .await;
    assert_eq!(other["repository"]["status"], "unbound");
    assert_eq!(other["cargo_dependencies"]["status"], "unbound");
    let widened = format!("{path}&path=/private/must-not-read");
    json(
        &prepared.app,
        request(&f, "GET", &widened, Some(&issued)),
        400,
    )
    .await;
}

#[tokio::test]
async fn config_drift_clears_live_repository_evidence_but_preserves_same_generation_design() {
    let f = Fixture::in_tmp(true);
    configure(&f, true);
    let prepared = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    let issued = cookie(&f, &prepared.app).await;
    let current = generation(&f, &prepared.app, &issued).await;
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    value["project_repository"]["cargo_dependencies"] = false.into();
    fs::write(&f.config, serde_json::to_vec(&value).unwrap()).unwrap();
    let result = json(
        &prepared.app,
        request(&f, "GET", &uri(PROJECT, current), Some(&issued)),
        200,
    )
    .await;
    assert_eq!(result["generation"], current);
    assert_eq!(result["architecture"]["status"], "ready");
    assert_eq!(result["repository"]["status"], "unavailable");
    assert!(result["repository"]["observed_at"].is_null());
    let drift = result["drift"].as_array().unwrap();
    assert!(!drift.is_empty());
    assert!(drift.iter().all(|row| row["status"] == "unverified"));
    assert_eq!(result["cargo_dependencies"]["status"], "unavailable");
    assert!(result["cargo_dependencies"]["graph"].is_null());
    assert!(result["cargo_dependencies"]["observed_at"].is_null());
    assert_eq!(result["rust_call_sites"]["status"], "disabled");
}

#[tokio::test]
async fn a_legacy_binding_added_by_a_library_caller_does_not_grant_session_repository_reads() {
    let f = Fixture::in_tmp(true);
    let repository = configure(&f, true);
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&f.config).unwrap()).unwrap();
    document
        .as_object_mut()
        .unwrap()
        .remove("project_repository");
    fs::write(&f.config, serde_json::to_vec(&document).unwrap()).unwrap();
    let prepared = prepare_with(f.options(), |_| async {
        Ok(test_support::verifier().await)
    })
    .await
    .unwrap();
    let legacy = studiolo_local_server::ProjectRepository::open(PROJECT, &repository)
        .unwrap()
        .with_cargo_dependencies();
    let app =
        studiolo_local_server::with_project_repository(prepared.app.clone(), Arc::new(legacy));
    let issued = cookie(&f, &app).await;
    let current = generation(&f, &app, &issued).await;
    let result = json(
        &app,
        request(&f, "GET", &uri(PROJECT, current), Some(&issued)),
        200,
    )
    .await;
    assert_eq!(result["repository"]["status"], "unbound");
    assert_eq!(result["cargo_dependencies"]["status"], "unbound");
    assert_eq!(result["architecture"]["status"], "ready");
}
