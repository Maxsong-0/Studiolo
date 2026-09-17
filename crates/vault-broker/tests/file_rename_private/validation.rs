use super::*;
#[test]
fn new_marker_and_private_terminal_fields_are_closed() {
    let (_fixture, mut broker, revision) = setup(b"value");
    let r = request("note.md", "target.md", revision, "model");
    let ticket = broker.prepare_file_operation(r, metadata()).unwrap();
    let state = crate::mutation::load(&broker).unwrap();
    let mut bad = state.clone();
    bad.rename_enabled = false;
    assert!(bad.validate(bad.pair_id).is_err());
    let mut bad = state.clone();
    bad.format_version = 5;
    assert!(bad.validate(bad.pair_id).is_err());
    let mut bad = state.clone();
    bad.file_operation.as_mut().unwrap().rename = None;
    assert!(bad.validate(bad.pair_id).is_err());
    let mut bad = state.clone();
    bad.file_operation.as_mut().unwrap().phase = FileOperationPhase::TargetInstalled;
    assert!(bad.validate(bad.pair_id).is_err());
    intent(&mut broker, &ticket);
    let result = broker.commit_file_operation(&ticket).unwrap();
    let request = ops::request::load(&broker, &result.operation_id)
        .unwrap()
        .unwrap();
    let terminal = ops::terminal::load(&broker, &request.frozen)
        .unwrap()
        .unwrap();
    let mut bad = terminal.clone();
    bad.receipt.effects.swap(0, 1);
    assert!(bad.validate(state.pair_id).is_err());
    let mut bad = terminal.clone();
    bad.receipt.effects[1] = bad.receipt.effects[0].clone();
    assert!(bad.validate(state.pair_id).is_err());
    let mut bad = terminal.clone();
    bad.rename = None;
    assert!(bad.validate(state.pair_id).is_err());
    let mut bad = terminal.clone();
    bad.format_version = 5;
    assert!(bad.validate(state.pair_id).is_err());
    let mut bad = terminal.clone();
    if let FileEffect::Confirmed {
        after: FilePresence::Present { revision: r, .. },
        ..
    } = &mut bad.receipt.effects[0]
    {
        *r = revision;
    }
    assert!(bad.validate(state.pair_id).is_err());
}
#[test]
fn malformed_dual_review_fields_cannot_be_confirmed_as_a_different_intent() {
    let (_fixture, mut broker, revision) = setup(b"value");
    pending(
        &mut broker,
        request("note.md", "target.md", revision, "review"),
        75,
    );
    let review = ops::maintenance::capture(&broker).unwrap().unwrap();
    let original = ops::maintenance::target(&review).unwrap();
    let mut wrong = original.clone();
    wrong.secondary = None;
    assert!(
        ops::maintenance::validate(
            &broker,
            &review,
            &wrong,
            MaintenanceDecision::PreserveInstalling
        )
        .is_err()
    );
    let mut wrong = original.clone();
    wrong.file_operation.as_mut().unwrap().effects.swap(0, 1);
    assert!(
        ops::maintenance::validate(
            &broker,
            &review,
            &wrong,
            MaintenanceDecision::PreserveInstalling
        )
        .is_err()
    );
    let mut wrong = original;
    wrong.secondary.as_mut().unwrap().current = MaintenanceCurrent::Missing;
    assert!(
        ops::maintenance::validate(
            &broker,
            &review,
            &wrong,
            MaintenanceDecision::PreserveInstalling
        )
        .is_err()
    );
}
#[test]
fn unsafe_names_cross_extension_and_new_links_are_rejected_before_any_issued() {
    for to in [
        "NOTE.md",
        "note.md/child.md",
        ".hidden/target.md",
        "target.pdf",
        "missing/target.md",
    ] {
        let (fixture, mut broker, revision) = setup(b"old");
        assert!(
            broker
                .prepare_file_operation(request("note.md", to, revision, "invalid"), metadata())
                .is_err(),
            "未拒绝非法目标：{to}"
        );
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
        assert!(!broker.write_status().unwrap().unresolved);
        assert_eq!(crate::mutation::load(&broker).unwrap().next_sequence, 1);
    }
    let (fixture, mut broker, revision) = setup(b"old");
    assert!(
        broker
            .prepare_file_operation(
                request(
                    "missing/note.md",
                    "target.md",
                    revision,
                    "missing-source-parent"
                ),
                metadata(),
            )
            .is_err()
    );
    assert_eq!(crate::mutation::load(&broker).unwrap().next_sequence, 1);
    assert!(!broker.write_status().unwrap().unresolved);
    assert!(!fixture.root.join("target.md").exists());
    let (fixture, mut broker, revision) = setup(b"old");
    fs::hard_link(fixture.root.join("note.md"), fixture.root.join("alias.md")).unwrap();
    assert!(
        broker
            .prepare_file_operation(
                request("note.md", "target.md", revision, "link"),
                metadata()
            )
            .is_err()
    );
}
