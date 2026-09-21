use crate::support::{
    allowed_workspace_dependencies, dependency_cycle, parse_manifest, workspace_manifests,
    workspace_root,
};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn dependency_scanner_detects_a_sneaky_cycle_fixture() {
    let manifests = [
        r#"
            [package]
            name = "crate-a"
            [dependencies]
            crate-b = { path = "../b" }
        "#,
        r#"
            [package]
            name = "crate-b"
            [dependencies]
            renamed-a = { package = "crate-a", path = "../a" }
        "#,
    ];
    let parsed = manifests
        .into_iter()
        .map(parse_manifest)
        .map(|manifest| (manifest.name.clone(), manifest))
        .collect::<BTreeMap<_, _>>();

    assert!(dependency_cycle(&parsed).is_some());
}

#[test]
fn dependency_scanner_includes_renamed_target_and_table_dependencies() {
    let manifest = parse_manifest(
        r#"
        [package]
        name = "fixture"
        [dependencies.renamed-broker]
        package = "studiolo-vault-broker"
        path = "../vault-broker"
        [target.'cfg(unix)'.dependencies.renamed-core]
        package = "studiolo-core"
        path = "../core"
        [target.'cfg(unix)'.build-dependencies]
        native = { package = "rustix", version = "1" }
        [dev-dependencies]
        test-only = "1"
    "#,
    );
    assert_eq!(
        manifest.dependencies,
        ["studiolo-vault-broker", "studiolo-core", "rustix"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
}

#[test]
fn workspace_dependencies_follow_the_acyclic_module_direction() {
    let manifests = workspace_manifests(&workspace_root());
    assert_eq!(manifests.len(), 23, "workspace crate inventory changed");
    assert!(
        dependency_cycle(&manifests).is_none(),
        "workspace dependency cycle: {:?}",
        dependency_cycle(&manifests)
    );

    let workspace_names = manifests.keys().cloned().collect::<BTreeSet<_>>();
    for manifest in manifests.values() {
        let allowed = allowed_workspace_dependencies(&manifest.name)
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let unexpected = manifest
            .dependencies
            .iter()
            .filter(|dependency| workspace_names.contains(*dependency))
            .filter(|dependency| !allowed.contains(dependency.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            unexpected.is_empty(),
            "{} has forbidden workspace dependencies: {unexpected:?}",
            manifest.name
        );
    }

    for read_side in [
        "studiolo-vault-index",
        "studiolo-gateway",
        "studiolo-audit-ledger",
        "studiolo-auth",
        "studiolo-auth-admin",
        "studiolo-host-config",
        "studiolo-runner-adapter",
        "studiolo-agent-server",
        "studiolo-sync-bridge",
        "studiolo-policy",
        "studiolo-process-safety",
    ] {
        assert!(
            !manifests[read_side]
                .dependencies
                .contains("studiolo-vault-broker"),
            "{read_side} must not acquire canonical Vault authority"
        );
    }
}

#[test]
fn filesystem_authority_dependencies_have_explicit_owners() {
    let manifests = workspace_manifests(&workspace_root());
    let canonical_filesystem_dependencies = ["cap-std", "cap-tempfile", "rustix"];

    for manifest in manifests.values() {
        let owned = canonical_filesystem_dependencies
            .iter()
            .filter(|dependency| manifest.dependencies.contains(**dependency))
            .copied()
            .collect::<Vec<_>>();
        if manifest.name == "studiolo-vault-broker" {
            assert_eq!(owned, canonical_filesystem_dependencies);
        } else if matches!(
            manifest.name.as_str(),
            "studiolo-auth-admin"
                | "studiolo-vault-admin"
                | "studiolo-host-config"
                | "studiolo-local-server"
                | "studiolo-sync-bridge"
                | "studiolo-backup"
                | "studiolo-observability"
                | "studiolo-sync-transport"
                | "studiolo-agent-server"
                | "studiolo-process-safety"
                | "studiolo-gateway"
        ) {
            // 仅本地配置/TTY、进程保护及Unix入口；源码门禁另限具体调用点和操作。
            assert_eq!(owned, ["rustix"]);
        } else {
            assert!(
                owned.is_empty(),
                "{} directly acquired canonical filesystem dependencies: {owned:?}",
                manifest.name
            );
        }
    }

    assert!(
        manifests["studiolo-control-db"]
            .dependencies
            .contains("rusqlite")
    );
    assert!(
        manifests["studiolo-runner-adapter"]
            .dependencies
            .contains("tempfile")
    );
    for manifest in manifests.values() {
        assert_eq!(
            manifest.dependencies.contains("notify"),
            manifest.name == "studiolo-local-server",
            "{} 的文件变更监视依赖必须由服务协调器独占",
            manifest.name
        );
        assert_eq!(
            manifest.dependencies.contains("rusqlite"),
            matches!(
                manifest.name.as_str(),
                "studiolo-control-db"
                    | "studiolo-audit-ledger"
                    | "studiolo-auth"
                    | "studiolo-supply"
                    | "studiolo-sync-bridge"
                    | "studiolo-policy"
                    | "studiolo-gateway"
                    | "studiolo"
            ),
            "{} must declare its SQLite ownership explicitly",
            manifest.name
        );
    }
    for forbidden in ["tempfile", "cap-std", "cap-tempfile", "rustix"] {
        assert!(
            !manifests["studiolo-audit-ledger"]
                .dependencies
                .contains(forbidden),
            "AuditLedger must not acquire arbitrary Vault authority, found {forbidden}"
        );
        assert!(
            !manifests["studiolo-auth"].dependencies.contains(forbidden),
            "Auth 不能获得任意 Vault 文件能力：{forbidden}"
        );
    }
}
