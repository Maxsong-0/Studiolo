use super::*;

#[test]
fn unpacking_restores_every_byte_and_the_previous_state_format() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    let before = loose_records(&fixture.data);
    pack(&mut broker, later());
    let preview = broker.archive_unpack_preview().unwrap();
    let done = broker.archive_unpack().unwrap();
    assert_eq!(preview, done);
    assert_eq!((done.packs, done.descriptors, done.replays), (6, 6, 6));
    assert!(!done.over_limit);
    assert_eq!(loose_records(&fixture.data), before);
    assert!(!fixture.data.join("archive-v1").exists());
    assert_eq!(fixture.state()["format_version"], 8);
    assert!(fixture.state().get("archive").is_none());
    for index in 0..6 {
        answers(&broker, index);
    }
    let (_, sealed) = checkpoint(&fixture, broker, "seal");
    assert_eq!(sealed.unwrap(), 12);
    // 还能再打一次，从第 1 段起。
    let mut broker = fixture.reopen();
    assert_eq!(pack(&mut broker, later()), 6);
    assert_eq!(fixture.state()["archive"]["segments"], 1);
}

/// 断在还原一半、状态没写回、状态已写回没删段：再跑、或者正常打开，都续得完。
#[test]
fn an_interrupted_unpack_resumes() {
    for point in 106..=108 {
        for again in [true, false] {
            let fixture = Fixture::new();
            let mut broker = fixture.open();
            notes(&mut broker, 6);
            let before = loose_records(&fixture.data);
            pack(&mut broker, later());
            fault(&broker, point);
            assert!(broker.archive_unpack().is_err(), "{point}");
            assert_eq!(all_records(&broker), before, "{point}");
            drop(broker);
            let mut broker = fixture.reopen();
            assert_eq!(all_records(&broker), before, "{point}");
            // 没写回状态的，正常打开当作打包没删完收口；写回了的，段清掉。
            assert_eq!(
                loose_records(&fixture.data).len(),
                if point == 108 { before.len() } else { 0 },
                "{point}"
            );
            if again {
                broker.archive_unpack().unwrap();
                assert_eq!(loose_records(&fixture.data), before, "{point}");
                assert_eq!(fixture.state()["format_version"], 8);
                assert!(fixture.segments().is_empty());
            }
            for index in 0..6 {
                answers(&broker, index);
            }
        }
    }
}

/// 段里的和已经在的散件不一样：不还原、不覆盖。
#[test]
fn unpacking_never_overwrites_a_different_loose_record() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 3);
    pack(&mut broker, later());
    let key = sha256(note(0).idempotency_key.as_bytes());
    let path = fixture.data.join(format!("idempotency/{key}.json"));
    fs::write(&path, b"{}").unwrap();
    assert!(broker.archive_unpack().is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{}");
    assert_eq!(fixture.state()["format_version"], 9);
}

#[test]
fn unpacking_a_store_that_was_never_packed_changes_nothing() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    let state = fs::read(fixture.data.join("writer-state.json")).unwrap();
    assert_eq!(broker.archive_unpack().unwrap().packs, 0);
    assert_eq!(
        fs::read(fixture.data.join("writer-state.json")).unwrap(),
        state
    );
}

/// 有没投递完的审计就不还原，什么都不动。
#[test]
fn unpacking_waits_until_nothing_is_unresolved() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    broker.mutate_recorded(note(5), metadata()).unwrap();
    let before = loose_records(&fixture.data);
    assert!(matches!(
        broker.archive_unpack(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.archive_unpack_preview(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(loose_records(&fixture.data), before);
    assert_eq!(fixture.state()["format_version"], 9);
}
