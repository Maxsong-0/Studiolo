use super::*;
use studiolo_vault_broker::ChangeKind;
use studiolo_workbench::{FileTree, WorkbenchReadModel};
fn generation(vault: VaultUuid, number: u64, sequence: u64, notes: &[(&str, &[u8])]) -> Generation {
    Generation {
        number,
        indexed_through: Some(sequence),
        invalidation_epoch: Some(1),
        model: WorkbenchReadModel::build(&[]).unwrap(),
        tree: FileTree::build(&[], &[]).unwrap(),
        revisions: notes
            .iter()
            .map(|(path, bytes)| {
                let path = VaultPath::parse(path).unwrap();
                let revision = Revision::compute(vault, &path, bytes);
                (
                    path,
                    Fingerprint {
                        revision,
                        bytes: bytes.len() as u64,
                    },
                )
            })
            .collect(),
    }
}
#[test]
fn startup_is_a_gap_and_only_successful_generation_differences_are_observations() {
    let vault = VaultUuid::new();
    let first = generation(vault, 1, 0, &[("a.md", b"old"), ("b.md", b"delete")]);
    let next = generation(vault, 2, 1, &[("a.md", b"new"), ("c.md", b"create")]);
    let mut log = ObservationLog::default();
    log.publish(None, &first, &[], true, Some(10)).unwrap();
    assert_eq!(log.rows.len(), 1);
    assert_eq!(log.rows[0].gap, Some("startup"));
    let known = ChangeEntry {
        id: "known".into(),
        sequence: 1,
        path: VaultPath::parse("a.md").unwrap(),
        kind: ChangeKind::Replaced,
        actor: None,
        previous_revision: None,
        revision: Some(next.revisions[&VaultPath::parse("a.md").unwrap()].revision),
        bytes: 3,
        at_ms: Some(11),
        counterpart: None,
    };
    log.publish(
        Some(&first),
        &next,
        std::slice::from_ref(&known),
        true,
        Some(12),
    )
    .unwrap();
    assert_eq!(log.rows.len(), 3);
    assert_eq!(log.rows[1].kind, ObservedKind::Deleted);
    assert_eq!(log.rows[2].kind, ObservedKind::Created);
    let mut outside = known;
    outside.sequence = 0;
    let all = changes(Some(&first), &next, &[outside], false, None);
    assert_eq!(all.len(), 4);
    assert_eq!(all[0].gap, Some("explanation_incomplete"));
    assert_eq!(all[1].kind, ObservedKind::Replaced);
}
#[test]
fn observation_retention_is_bounded_and_counter_exhaustion_is_atomic() {
    let vault = VaultUuid::new();
    let first = generation(vault, 1, 0, &[]);
    let mut log = ObservationLog::default();
    for _ in 0..OBSERVATION_LIMIT + 3 {
        log.publish(None, &first, &[], true, None).unwrap();
    }
    assert_eq!(log.rows.len(), OBSERVATION_LIMIT);
    assert_eq!(log.lost_through, 3);
    assert_eq!(log.rows.front().unwrap().index, 4);
    log.last_index = u64::MAX;
    let size = log.rows.len();
    assert!(log.publish(None, &first, &[], true, None).is_err());
    assert_eq!(log.rows.len(), size);
    assert_ne!(log.epoch, ObservationLog::default().epoch);
}
