use super::support::{Fixture, inventory, path};
use std::{fs, io::ErrorKind};
use studiolo_core::{AttemptId, Fence, TaskId};
use studiolo_vault_broker::{ArtifactCandidate, ArtifactCommit, Mutation, VaultWriter};
use studiolo_vault_broker::{BrokerError, FsVaultBroker};

/// 活写者复核固定句柄：canonical 根被换报根已变，其余被换节点报配对不符。
fn live_rejection(node: &str, error: &BrokerError) -> bool {
    match node {
        "vault" => matches!(error, BrokerError::RootChanged),
        _ => matches!(error, BrokerError::WriterBindingMismatch),
    }
}

pub(super) fn artifacts() -> ArtifactCommit {
    ArtifactCommit {
        task_id: TaskId::new(),
        attempt_id: AttemptId::new(),
        fence: Fence::new(1).unwrap(),
        audit_intent: "fictional-writer-test".into(),
        idempotency_key: "writer-artifacts".into(),
        artifacts: vec![ArtifactCandidate {
            name: path("result.md"),
            content: b"result".to_vec(),
        }],
    }
}

#[test]
fn replacing_a_pinned_root_control_marker_or_data_stops_all_installation() {
    for node in ["vault", "control", "marker", "data"] {
        let fixture = Fixture::initialized();
        let mut broker = fixture.open();
        let target = match node {
            "vault" => fixture.vault.clone(),
            "control" => fixture.vault.join(".workbench"),
            "marker" => fixture.marker(),
            _ => fixture.data.clone(),
        };
        let moved = fixture.directory.path().join("original");
        fs::rename(&target, &moved).unwrap();
        if node == "marker" {
            // UUID 和字节保持相同，只有文件身份发生变化。
            fs::copy(&moved, &target).unwrap();
        } else {
            fs::create_dir(&target).unwrap();
        }
        let before = inventory(fixture.directory.path());
        let status = broker.write_status();
        assert!(
            matches!(&status, Err(error) if live_rejection(node, error)),
            "status accepted replaced {node}: {:?}",
            status.err()
        );
        let mutation = broker.mutate(Mutation::create(
            path("escape.md"),
            b"forbidden".to_vec(),
            "after-replacement",
        ));
        assert!(
            matches!(&mutation, Err(error) if live_rejection(node, error)),
            "mutation accepted replaced {node}: {:?}",
            mutation.err()
        );
        let commit = broker.commit_attempt_artifacts(artifacts());
        assert!(
            matches!(&commit, Err(error) if live_rejection(node, error)),
            "commit accepted replaced {node}: {:?}",
            commit.err()
        );
        assert_eq!(
            inventory(fixture.directory.path()),
            before,
            "wrote after {node} replacement"
        );
        drop(broker);
        // 重开时：空的新 canonical 根缺控制目录；其余替换使两侧配对记录或 marker 不符。
        let reopened = FsVaultBroker::open_existing(&fixture.vault, &fixture.data);
        let rejected = match node {
            "vault" => matches!(
                &reopened,
                Err(BrokerError::Io {
                    operation: "inspect writer layout",
                    source,
                }) if source.kind() == ErrorKind::NotFound
            ),
            _ => matches!(&reopened, Err(BrokerError::WriterBindingMismatch)),
        };
        assert!(rejected, "reopened replaced {node}: {:?}", reopened.err());
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}
