use super::*;
use std::os::unix::fs::symlink;

fn orphan(f: &Fixture, id: usize, bytes: &[u8]) -> PathBuf {
    let path = f
        .root
        .join(".workbench/staging")
        .join(format!("{id:064x}.stage"));
    fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn ready_or_prepared_pruning_keeps_owned_stage_and_does_not_touch_evidence() {
    for pending in [false, true] {
        let f = Fixture::new();
        let mut broker = if pending {
            f.interrupted(1).0
        } else {
            f.initialize()
        };
        let abandoned = orphan(&f, 1, b"abandoned");
        let evidence = f.data.join("snapshots/recovery-retained.bin");
        fs::write(&evidence, b"retained").unwrap();
        let status = broker.prune_stages().unwrap();
        assert_eq!(status.removed, 1);
        assert_eq!(status.retained, usize::from(pending));
        assert!(!abandoned.exists());
        assert_eq!(fs::read(evidence).unwrap(), b"retained");
        if pending {
            assert!(f.stage(&broker).exists());
        }
    }
}

#[test]
fn malformed_linked_or_oversized_stage_makes_preflight_non_destructive() {
    for damage in ["name", "symlink", "hardlink", "large"] {
        let f = Fixture::new();
        let mut broker = f.initialize();
        let good = orphan(&f, 1, b"good");
        let bad = f
            .root
            .join(".workbench/staging")
            .join(format!("{:064x}.stage", 2));
        match damage {
            "name" => fs::write(bad.with_extension("wrong"), b"bad").unwrap(),
            "symlink" => symlink(&good, &bad).unwrap(),
            "hardlink" => fs::hard_link(&good, &bad).unwrap(),
            _ => {
                fs::File::create(&bad)
                    .unwrap()
                    .set_len(crate::blob::OBJECT_LIMIT + 1)
                    .unwrap();
            }
        }
        assert!(broker.prune_stages().is_err());
        assert!(good.exists());
    }
}

#[test]
fn stage_count_budget_fails_before_any_deletion() {
    let f = Fixture::new();
    let mut broker = f.initialize();
    for id in 0..65 {
        orphan(&f, id, b"");
    }
    assert!(matches!(
        broker.prune_stages(),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(
        fs::read_dir(f.root.join(".workbench/staging"))
            .unwrap()
            .count(),
        65
    );
}

#[test]
fn pending_unknown_stages_cannot_be_reclaimed() {
    for point in [2, 3, 4] {
        let f = Fixture::new();
        let (mut broker, _) = f.interrupted(point);
        let file = orphan(&f, 1, b"keep");
        assert!(matches!(
            broker.prune_stages(),
            Err(BrokerError::MutationUnresolved)
        ));
        assert!(file.exists());
    }
}

#[test]
fn stage_directory_replacement_is_not_adopted() {
    let f = Fixture::new();
    let mut broker = f.initialize();
    let stages = f.root.join(".workbench/staging");
    fs::rename(&stages, f.root.join(".workbench/retained")).unwrap();
    fs::create_dir(&stages).unwrap();
    let file = orphan(&f, 1, b"foreign");
    assert!(broker.prune_stages().is_err());
    assert_eq!(fs::read(file).unwrap(), b"foreign");
}
