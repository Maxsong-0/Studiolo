//! 写者由可信装配打开一次；HTTP、监视与协调器不能补造第二个 owner 或写入口。

use crate::support::{rust_sources, source_violations, workspace_root};
use std::{fs, path::Path};

const INPUT: &str = "startup/canonical_input.rs";
const MAINTENANCE_INPUT: &str = "maintenance/inputs.rs";

fn compact(source: &str) -> String {
    source
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")")
}

// 此词法门禁补充原权限扫描，不替代运行时租约、根身份与取消回归。
fn boundary_violations(path: &Path, source: &str) -> Vec<&'static str> {
    let mut violations = Vec::new();
    let mut remaining = compact(source);
    let owner = path.ends_with("coordinator.rs")
        || path
            .ancestors()
            .skip(1)
            .any(|parent| parent.ends_with("coordinator"));
    let board_owner = path.ends_with("board/owner.rs");
    let board_root = path.ends_with("board/mod.rs");
    let maintenance_input = path.ends_with(MAINTENANCE_INPUT);
    let input = path.ends_with(INPUT) || maintenance_input;
    if remaining.contains("FsVaultBroker") && !input && !owner && !board_owner {
        violations.push("writer_type");
    }
    if input {
        // 仅此固定的既有绑定工厂获准；再次调用或改用初始化工厂仍留在余串。
        remaining = remaining.replacen("FsVaultBroker::open_existing(", "", 1);
    }
    if maintenance_input {
        remaining = remaining.replacen(
            "SqliteAuditLedger::open_existing(config.audit_directory())",
            "",
            1,
        );
    }
    if remaining.contains("FsVaultBroker::")
        || ((owner || input)
            && ["::open(", "::open_existing("]
                .iter()
                .any(|value| remaining.contains(value)))
    {
        violations.push("writer_factory");
    }
    if !owner && !board_owner && remaining.contains(".owner") {
        violations.push("owner_escape");
    }
    if path.ends_with("coordinator/content.rs") {
        // 唯一人写正文入口；只放行固定元数据写入及CAS拒绝，不授予任意写回调。
        for allowed in [
            "Mutation::replace(",
            "broker.mutate_recorded(mutation,metadata)",
            "broker.check_precondition_recorded(mutation,metadata)",
        ] {
            remaining = remaining.replacen(allowed, "", 1);
        }
    }
    if path.ends_with("coordinator/sync_bridge.rs") {
        // 双向 Bridge 的唯一 canonical 接缝（T-132 P1，ADR-025 D8）：只放行固定的
        // 「外部：Obsidian」条件写／流式写形态，主体与幂等键都由调用方按执行器原键给出。
        for allowed in [
            "Mutation::create(path.clone(),content,idempotency_key)",
            "Mutation::replace(path.clone(),content,expected,idempotency_key)",
            "broker.mutation_replay_recorded(&mutation,&metadata)",
            "broker.mutate_recorded(mutation,metadata)",
            "broker.prepare_stream_mutation(request,&source,limits,metadata)",
            "broker.commit_stream_mutation(ticket)",
            "broker.cancel_stream_mutation(ticket)",
        ] {
            remaining = remaining.replacen(allowed, "", 1);
        }
    }
    if path.ends_with("coordinator/content/restore.rs") {
        for allowed in [
            "broker.prepare_recorded_restore(mutation,metadata)",
            "broker.confirm_recorded_restore_intent(&ticket,offset,&hash)",
            "broker.commit_recorded_restore(ticket)",
            "broker.cancel_recorded_restore(ticket)",
        ] {
            remaining = remaining.replacen(allowed, "", 1);
        }
    }
    if board_owner {
        // 黑板仅借用原writer；此精确代理不授予任何工厂或环境文件访问。
        for allowed in [
            "implVaultWriterforSharedBroker{",
            "implVaultSnapshotterforSharedBroker{",
            "self.access(|b|b.mutate(mutation))",
            "fncommit_attempt_artifacts(&mutself,commit:ArtifactCommit)->Result<ArtifactCommitReceipt,BrokerError>",
            "self.access(|b|b.commit_attempt_artifacts(commit))",
        ] {
            remaining = remaining.replacen(allowed, "", 1);
        }
    }
    if board_root {
        for allowed in [
            "usestudiolo_task_engine::PersistentTaskEngine;",
            "typeEngine=PersistentTaskEngine<owner::SharedBroker,SharedAudit>;",
            "PersistentTaskEngine::new(control,broker,SharedAudit(audit.clone()))",
        ] {
            remaining = remaining.replacen(allowed, "", 1);
        }
    }
    for forbidden in [
        "VaultWriter",
        "VaultSnapshotter",
        "Mutation::",
        ".mutate(",
        "::mutate(",
        ".mutate_recorded(",
        ".check_precondition_recorded(",
        ".prepare_recorded_restore(",
        ".confirm_recorded_restore_intent(",
        ".commit_recorded_restore(",
        ".cancel_recorded_restore(",
        "commit_attempt_artifacts(",
        "M0Workbench",
        "PersistentTaskEngine",
    ] {
        if remaining.contains(forbidden) {
            violations.push("write_capability");
        }
    }
    violations
}

#[test]
fn only_trusted_startup_and_offline_maintenance_open_the_existing_writer() {
    let root = workspace_root().join("crates/local-server/src");
    for path in rust_sources(&root) {
        let source = fs::read_to_string(&path).unwrap();
        let violations = boundary_violations(&path, &source);
        assert!(
            violations.is_empty(),
            "写者边界逸出：{path:?} {violations:?}"
        );
    }
    let input = compact(&fs::read_to_string(root.join(INPUT)).unwrap());
    assert_eq!(input.matches("FsVaultBroker::open_existing(").count(), 1);
    assert!(input.contains(
        "FsVaultBroker::open_existing(roots.vault_directory(),roots.writer_data_directory()"
    ));
    assert!(input.contains(
        "broker.validate_root_identities(roots.vault_identity(),roots.writer_data_identity())"
    ));
    assert!(input.contains("CanonicalWorkbench::prepare_with_audit(self.broker,self.roots,audit)"));
    assert!(!input.contains("PathBuf"));
}

#[test]
fn boundary_regression_detects_second_owner_alias_and_write_capabilities() {
    let root = workspace_root().join("crates/local-server/src");
    let path = root.join(INPUT);
    let source = fs::read_to_string(&path).unwrap();
    assert!(boundary_violations(&path, &source).is_empty());
    for extra in [
        "FsVaultBroker::open(root, alternate_data);",
        "FsVaultBroker::open_existing(root, alternate_data);",
        "use studiolo_vault_broker::FsVaultBroker as Other; Other::open_existing(root, data);",
        "use studiolo_vault_broker::VaultWriter; broker.mutate(mutation);",
        "broker.commit_attempt_artifacts(task, attempt, artifacts);",
        "broker.mutate_recorded(mutation, metadata);",
        "broker.commit_recorded_restore(ticket);",
    ] {
        assert!(
            !boundary_violations(&path, &format!("{source}\n{extra}")).is_empty(),
            "{extra}"
        );
    }
    for misplaced in [
        "canonical.rs",
        "coordinator.rs",
        "routing/canonical_writer.rs",
        "watcher.rs",
    ] {
        assert!(
            !boundary_violations(&root.join(misplaced), &source).is_empty(),
            "{misplaced}"
        );
    }
    assert!(
        !boundary_violations(
            &root.join("canonical.rs"),
            "state.workbench.core.owner.lock();"
        )
        .is_empty()
    );
}

#[test]
fn coordinator_and_http_keep_the_original_filesystem_authority_scanner() {
    let root = workspace_root();
    let source_root = root.join("crates/local-server/src");
    let mut paths = rust_sources(&source_root.join("coordinator"));
    paths.extend(rust_sources(&source_root.join("routing")));
    paths.extend(
        [
            "coordinator.rs",
            "canonical.rs",
            "routing.rs",
            "refresh.rs",
            "watcher.rs",
            INPUT,
        ]
        .map(|path| source_root.join(path)),
    );
    for path in paths {
        let source = fs::read_to_string(&path).unwrap();
        assert!(
            source_violations("studiolo-local-server", &path, &source).is_empty(),
            "{path:?}"
        );
        for extra in [
            "std::fs::read_to_string(path);",
            "std::fs::write(path, bytes);",
            "use std::{fs::{write as save}}; save(path, bytes);",
            "let root = Dir::open_ambient_dir(path, ambient_authority());",
        ] {
            assert!(
                !source_violations(
                    "studiolo-local-server",
                    &path,
                    &format!("{source}\n{extra}")
                )
                .is_empty(),
                "原权限扫描必须仍拒绝 {path:?} 的 {extra}"
            );
        }
    }
}

#[test]
fn canonical_router_receives_a_completed_owner_and_keeps_legacy_sources_distinct() {
    let root = workspace_root().join("crates/local-server/src");
    let routing = compact(&fs::read_to_string(root.join("routing/canonical_writer.rs")).unwrap());
    assert!(routing.contains("workbench:Arc<CanonicalWorkbench>"));
    assert!(routing.contains("workbench.revalidate()?"));
    assert!(routing.contains("assets.root_identity()?!=workbench.core.roots.assets_identity()"));
    assert!(routing.contains("source:Source::Writer(Arc::clone(&workbench))"));
    assert!(routing.contains("canonical:Some(CanonicalSource::Writer(workbench))"));
    assert!(routing.contains("Some(access),Some(sessions)"));
    for forbidden in [
        "FsVaultBroker",
        "PathBuf",
        "&Path",
        "open_existing(",
        "load(",
    ] {
        assert!(
            !routing.contains(forbidden),
            "路由不接受未批准根或重建owner：{forbidden}"
        );
    }
    let legacy = compact(&fs::read_to_string(root.join("routing.rs")).unwrap());
    assert!(legacy.contains("Source::Replica(Arc::new(vault))"));
    assert!(legacy.contains("canonical.map(CanonicalSource::Reader)"));
    let refresh = compact(&fs::read_to_string(root.join("refresh.rs")).unwrap());
    assert!(refresh.contains("Source::Writer(workbench)=>workbench.refresh().await"));
    assert!(refresh.contains("Source::Replica(vault)=>"));
    let owner = compact(&fs::read_to_string(root.join("coordinator.rs")).unwrap());
    let preparation =
        compact(&fs::read_to_string(root.join("coordinator/preparation.rs")).unwrap());
    assert!(
        preparation
            .contains("pubasyncfnprepare(broker:FsVaultBroker,roots:Arc<CanonicalServingRoots>")
    );
    assert!(preparation.contains("owner:Mutex::new(Some(broker))"));
    assert!(owner.contains("owner:Mutex<Option<FsVaultBroker>>"));
    assert!(!owner.contains("pubstructCore"));
}

#[test]
fn blackboard_proxy_and_executor_do_not_gain_extra_writer_factories() {
    let root = workspace_root().join("crates/local-server/src");
    for name in ["board/owner.rs", "board/mod.rs", "board/executor/mod.rs"] {
        let path = root.join(name);
        let source = fs::read_to_string(&path).unwrap();
        assert!(boundary_violations(&path, &source).is_empty(), "{name}");
        for extra in [
            "broker.mutate(arbitrary);",
            "broker.commit_attempt_artifacts(other);",
            "FsVaultBroker::open(root, data);",
            "use studiolo_vault_broker::VaultSnapshotter;",
            "PersistentTaskEngine::new(control,broker,SharedAudit(audit.clone()));",
        ] {
            assert!(
                !boundary_violations(&path, &format!("{source}\n{extra}")).is_empty(),
                "{name}: {extra}"
            );
        }
    }
}
