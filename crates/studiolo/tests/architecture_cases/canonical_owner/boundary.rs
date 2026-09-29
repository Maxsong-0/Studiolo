//! canonical 写者接缝的词法规则；只返回违规分类，测试在父模块。

use super::{INPUT, MAINTENANCE_INPUT, compact};
use std::path::Path;

// 此词法门禁补充原权限扫描，不替代运行时租约、根身份与取消回归。
pub(super) fn boundary_violations(path: &Path, source: &str) -> Vec<&'static str> {
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
    if path.ends_with("coordinator/sync_bridge.rs") {
        // 双向 Bridge 的唯一 canonical 接缝（ADR-025 D8）：八条「外部：Obsidian」写形态各一次，加一条外 stage 只读附件来源。
        for allowed in [
            "Mutation::create(path.clone(),content,idempotency_key)",
            "Mutation::replace(path.clone(),content,expected,idempotency_key)",
            "broker.mutation_replay_recorded(&mutation,&metadata)",
            "broker.mutate_recorded(mutation,metadata)",
            "broker.stream_mutation_replay(&request,&metadata)",
            "broker.prepare_stream_mutation(request,&source,limits,metadata)",
            "broker.commit_stream_mutation(ticket)",
            "broker.cancel_stream_mutation(ticket)",
            "BlobSource::open(stage_root,stage_object.clone())",
        ] {
            remaining = remaining.replacen(allowed, "", 1);
        }
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
        ".stream_mutation_replay(",
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
