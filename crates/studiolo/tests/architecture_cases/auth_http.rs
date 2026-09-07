//! HTTP只能装配已存在的Auth，不获得SQL、种子或首次登记能力。

use crate::support::{rust_sources, workspace_root};
use std::fs;

#[path = "canonical_owner.rs"]
mod canonical_owner;

#[test]
fn session_http_has_no_storage_seed_or_identity_factory_authority() {
    let root = workspace_root().join("crates/local-server/src");
    let mut paths = rust_sources(&root.join("auth_http"));
    paths.push(root.join("auth_http.rs"));
    let source = paths
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<String>();
    for forbidden in [
        "SessionManager::open",
        "SessionManager::migrate",
        "Connection::",
        "rusqlite::",
        "TotpEnrollment",
        "TotpSeed",
        "fixture_verifier",
        "AccessIdentity::",
        ".issue(",
        ".enroll(",
        "std::env::",
        "std::fs::",
    ] {
        assert!(
            !source.contains(forbidden),
            "HTTP不得获得该能力：{forbidden}"
        );
    }
    assert!(source.contains(".issue_enrolled("));
    assert!(source.contains(".check_with_csrf("));
}

#[test]
fn session_composition_is_explicit_and_does_not_change_the_existing_binary_mode() {
    let root = workspace_root().join("crates/local-server/src");
    let routing = fs::read_to_string(root.join("routing.rs")).unwrap();
    let main = fs::read_to_string(root.join("main.rs")).unwrap();
    let startup = fs::read_to_string(root.join("startup.rs")).unwrap();
    assert!(routing.contains("pub fn router_with_sessions("));
    assert!(routing.contains("sessions: Arc<SessionManager>"));
    assert!(routing.contains("if let Some(sessions) = &state.sessions"));
    // 分别检查旧入口，不能由新写者入口中偶然相同的参数片段代替。
    for (start, end, arguments) in [
        (
            "pub fn router(",
            "pub fn router_with_access(",
            "Endpoint::Loopback(port),None,None,None)",
        ),
        (
            "pub fn router_with_access(",
            "pub fn router_with_sessions(",
            "Endpoint::Loopback(port),Some(access),None,None)",
        ),
    ] {
        let function = routing
            .split_once(start)
            .unwrap()
            .1
            .split_once(end)
            .unwrap()
            .0;
        let compact = function
            .split_whitespace()
            .collect::<String>()
            .replace(",)", ")");
        assert!(compact.contains("configured_router("));
        assert!(compact.contains(arguments));
        assert!(!compact.contains("CanonicalWorkbench"));
    }
    assert!(!main.contains("router_with_sessions"));
    assert!(!main.contains("SessionManager"));
    assert!(main.contains("PreparedServer::prepare(options)"));
    assert!(startup.contains("prepare_with(options, AccessVerifier::connect)"));
    assert!(startup.contains("Config::load(&session.config)"));
    assert!(startup.contains("SqliteAuditLedger::open_existing"));
    assert!(startup.contains("SessionManager::open_existing"));
    assert!(startup.contains(".require_http_ready()"));
    assert!(startup.contains("router_with_sessions("));
    for forbidden in [
        "TotpSeed",
        "TotpEnrollment",
        "fixture_verifier",
        "::migrate",
        "std::env::",
        "std::fs::",
    ] {
        assert!(
            !startup.contains(forbidden),
            "启动层不得获得该能力：{forbidden}"
        );
    }
}

#[test]
fn canonical_http_only_uses_the_selected_existing_read_capability() {
    let root = workspace_root().join("crates/local-server/src");
    let source = fs::read_to_string(root.join("canonical.rs")).unwrap();
    for forbidden in [
        "FsVaultBroker",
        "std::fs",
        "std::env",
        "open_existing",
        ".mutate(",
    ] {
        assert!(
            !source.contains(forbidden),
            "单页HTTP不得获得该能力：{forbidden}"
        );
    }
    assert!(source.contains("CanonicalSource::Reader(vault)"));
    assert!(source.contains("CanonicalSource::Writer(workbench)"));
    let compact = source.split_whitespace().collect::<String>();
    assert!(compact.contains("vault.read_file(&path,MAX_BYTES)"));
    assert!(compact.contains("workbench.read_file(&path,MAX_BYTES)"));
    let routing = fs::read_to_string(root.join("routing.rs")).unwrap();
    assert!(routing.contains("view.root_identity()? != canonical.root_identity()?"));
    assert!(routing.contains("if state.canonical.is_some()"));
}
