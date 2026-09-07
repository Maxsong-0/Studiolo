use super::support::{Fixture, inventory, path};
use std::fs;
use studiolo_core::Revision;
use studiolo_vault_broker::{BrokerError, Mutation, VaultReader, VaultWriter};

#[test]
fn watermarks_survive_reopen_and_replay_does_not_claim_current_content() {
    let fixture = Fixture::initialized();
    let mut broker = fixture.open();
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
    assert!(!broker.write_status().unwrap().unresolved);
    let request = Mutation::create(path("note.md"), b"first".to_vec(), "first");
    let receipt = broker.mutate(request.clone()).unwrap();
    let one = broker.write_status().unwrap().committed_through;
    assert_eq!(one, 1);
    let second = broker
        .mutate(Mutation::replace(
            path("note.md"),
            b"second".to_vec(),
            receipt.revision,
            "second",
        ))
        .unwrap();
    let two = broker.write_status().unwrap().committed_through;
    assert_eq!(two, 2);
    assert_eq!(broker.mutate(request.clone()).unwrap(), receipt);
    assert_eq!(broker.write_status().unwrap().committed_through, two);
    assert_eq!(
        broker.read(&path("note.md")).unwrap().revision,
        second.revision
    );
    drop(broker);
    // 完成回执描述原提交；带外变化不能被重开或重放覆盖。
    fs::write(fixture.vault.join("note.md"), b"external").unwrap();
    let before = inventory(&fixture.vault);
    let mut reopened = fixture.open();
    assert_eq!(reopened.write_status().unwrap().committed_through, two);
    assert!(!reopened.write_status().unwrap().unresolved);
    assert_eq!(reopened.mutate(request).unwrap(), receipt);
    assert_eq!(
        reopened.read(&path("note.md")).unwrap().content,
        b"external"
    );
    assert_eq!(reopened.write_status().unwrap().committed_through, two);
    assert_eq!(inventory(&fixture.vault), before);
}

#[test]
fn explicit_cas_rejection_does_not_advance_commit_watermark() {
    let fixture = Fixture::initialized();
    let mut broker = fixture.open();
    broker
        .mutate(Mutation::create(
            path("note.md"),
            b"actual".to_vec(),
            "initial",
        ))
        .unwrap();
    let before = broker.write_status().unwrap().committed_through;
    let wrong = Revision::compute(broker.vault_uuid(), &path("note.md"), b"wrong");
    assert!(matches!(
        broker.mutate(Mutation::replace(
            path("note.md"),
            b"rejected".to_vec(),
            wrong,
            "rejected"
        )),
        Err(BrokerError::RevisionConflict { .. })
    ));
    assert_eq!(broker.write_status().unwrap().committed_through, before);
    assert!(!broker.write_status().unwrap().unresolved);
    drop(broker);
    assert_eq!(
        fixture.open().write_status().unwrap().committed_through,
        before
    );
}

#[test]
fn l0_history_keeps_original_bytes_including_non_utf8_and_external_changes() {
    let fixture = Fixture::initialized();
    let mut broker = fixture.open();
    let original = b"\xef\xbb\xbfline\r\n\0\xff".to_vec();
    let first = broker
        .mutate(Mutation::create(
            path("note.md"),
            original.clone(),
            "original",
        ))
        .unwrap();
    broker
        .mutate(Mutation::replace(
            path("note.md"),
            b"second".to_vec(),
            first.revision,
            "second",
        ))
        .unwrap();
    let history = fixture
        .data
        .join("history")
        .join(first.revision.to_string());
    assert_eq!(fs::read(&history).unwrap(), original);
    let outside = b"external\r\n\0\x80";
    fs::write(fixture.vault.join("note.md"), outside).unwrap();
    let observed = broker.read(&path("note.md")).unwrap();
    let receipt = broker
        .mutate(Mutation::replace(
            path("note.md"),
            b"third".to_vec(),
            observed.revision,
            "third",
        ))
        .unwrap();
    assert_eq!(receipt.previous_revision, Some(observed.revision));
    assert_eq!(
        fs::read(
            fixture
                .data
                .join("history")
                .join(observed.revision.to_string())
        )
        .unwrap(),
        outside
    );
    drop(broker);
    drop(fixture.open());
    assert_eq!(fs::read(history).unwrap(), original);
}
