//! 双向 Bridge 接缝（`coordinator/sync_bridge.rs`）的红臂：八条固定写形态各一次加一条外 stage
//! 只读附件来源放行，多一处、换形态、换文件名都红。从 `canonical_owner.rs` 拆出只为压住行数。

use super::boundary_violations;
use crate::support::workspace_root;
use std::fs;

#[test]
fn the_bridge_seam_admits_only_its_eight_write_shapes_and_one_blob_source() {
    let root = workspace_root().join("crates/local-server/src");
    let path = root.join("coordinator/sync_bridge.rs");
    let source = fs::read_to_string(&path).unwrap();
    assert!(boundary_violations(&path, &source).is_empty());
    // 与 W3 任务书第 2 节逐字一致的八条形态，各一次，加一条外 stage 附件来源。
    let allowed = [
        "Mutation::create(path.clone(), content, idempotency_key)",
        "Mutation::replace(path.clone(), content, expected, idempotency_key)",
        "broker.mutation_replay_recorded(&mutation, &metadata)",
        "broker.mutate_recorded(mutation, metadata)",
        "broker.stream_mutation_replay(&request, &metadata)",
        "broker.prepare_stream_mutation(request, &source, limits, metadata)",
        "broker.commit_stream_mutation(ticket)",
        "broker.cancel_stream_mutation(ticket)",
        "let source = studiolo_vault_broker::BlobSource::open(stage_root, stage_object.clone())?;",
    ];
    // 真实接缝里八条形态已各出现一次，夹具只用这九行本身：各一次必须放行，再多一次就红。
    let seam = allowed.join("\n");
    assert!(
        boundary_violations(&path, &seam).is_empty(),
        "八条固定形态各一次加一条附件来源必须放行"
    );
    assert!(
        !boundary_violations(&path, &format!("{source}\n{seam}")).is_empty(),
        "真实接缝已含八条形态，再各加一次必须红"
    );
    for extra in [
        "broker.stream_mutation_replay(&request, &metadata);",
        "broker.stream_mutation_replay(&other, &metadata);",
        "broker.mutate(mutation);",
        "broker.mutate_recorded(other, metadata);",
        "Mutation::replace(path.clone(), content, expected, idempotency_key);",
        "FsVaultBroker::open(root, data);",
        "FsVaultBroker::open_existing(root, data);",
        "BlobSource::open(other_root, object);",
        "use studiolo_vault_broker::VaultWriter;",
        "broker.commit_attempt_artifacts(commit);",
    ] {
        assert!(
            !boundary_violations(&path, &format!("{seam}\n{extra}")).is_empty(),
            "{extra}"
        );
    }
    // 同一份接缝代码放到 sync_plane 下：写者边界不许逸出。
    assert!(!boundary_violations(&root.join("sync_plane/driver.rs"), &seam).is_empty());
}
