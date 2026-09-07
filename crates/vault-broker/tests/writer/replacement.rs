use super::support::{Fixture, inventory, path};
use std::fs;
use studiolo_core::{AttemptId, Fence, TaskId};
use studiolo_vault_broker::FsVaultBroker;
use studiolo_vault_broker::{ArtifactCandidate, ArtifactCommit, Mutation, VaultWriter};

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
        assert!(
            broker.write_status().is_err(),
            "status accepted replaced {node}"
        );
        assert!(
            broker
                .mutate(Mutation::create(
                    path("escape.md"),
                    b"forbidden".to_vec(),
                    "after-replacement"
                ))
                .is_err()
        );
        assert!(broker.commit_attempt_artifacts(artifacts()).is_err());
        assert_eq!(
            inventory(fixture.directory.path()),
            before,
            "wrote after {node} replacement"
        );
        drop(broker);
        assert!(FsVaultBroker::open_existing(&fixture.vault, &fixture.data).is_err());
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}
