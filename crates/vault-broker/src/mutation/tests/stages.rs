use super::*;

#[test]
fn changed_missing_or_linked_prepared_stage_never_installs() {
    for damage in ["content", "missing", "symlink", "hardlink", "oversized"] {
        let fixture = Fixture::new();
        let mut broker = fixture.initialize();
        interrupt(&broker, 1);
        assert!(broker.mutate(request()).is_err());
        let stage = fixture.stage(&broker);
        let preserved = fixture._temp.path().join("preserved");
        match damage {
            "content" => fs::write(&stage, b"changed").unwrap(),
            "missing" => fs::remove_file(&stage).unwrap(),
            "oversized" => {
                fs::write(&stage, vec![0u8; crate::writer::disk::CONTENT_LIMIT + 1]).unwrap()
            }
            _ => {
                fs::rename(&stage, &preserved).unwrap();
                if damage == "symlink" {
                    std::os::unix::fs::symlink(&preserved, &stage).unwrap();
                } else {
                    fs::hard_link(&preserved, &stage).unwrap();
                }
            }
        }
        assert!(broker.mutate(request()).is_err(), "accepted {damage}");
        assert!(broker.write_status().unwrap().unresolved);
        assert!(!fixture.root.join("note.md").exists());
        drop(broker);
        let mut reopened = fixture.open();
        assert!(reopened.mutate(request()).is_err());
        assert!(!fixture.root.join("note.md").exists());
    }
}

#[test]
fn orphan_stage_budget_blocks_without_claiming_pending_installation() {
    let fixture = Fixture::new();
    let mut broker = fixture.initialize();
    for index in 0..64 {
        fs::write(
            fixture
                .root
                .join(".workbench/staging")
                .join(format!("{index:064x}.stage")),
            b"orphan",
        )
        .unwrap();
    }
    assert!(matches!(
        broker.mutate(request()),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(!fixture.root.join("note.md").exists());
    assert!(!broker.write_status().unwrap().unresolved);
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
}
