use super::*;

/// 每个断点：断开、（进程死了）重开，记录一条不丢、一条不重，接着打完。
/// 100 段还没写；101 段已落、状态没存；102 状态已存、散件没删；103 一包删了两条；104 删过半；105 删完没落盘。
#[test]
fn every_crash_point_resumes_without_losing_or_repeating_a_record() {
    for point in 100..=105 {
        for reopened in [true, false] {
            let fixture = Fixture::new();
            let mut broker = fixture.open();
            notes(&mut broker, 6);
            let before = all_records(&broker);
            fault(&broker, point);
            assert!(
                broker
                    .archive_step(later(), &mut ArchiveCursor::default())
                    .is_err(),
                "{point}"
            );
            assert_eq!(all_records(&broker), before, "断开那一刻 {point}");
            let committed = point >= 102;
            assert_eq!(fixture.state()["archive"].is_object(), committed, "{point}");
            if reopened {
                drop(broker);
                broker = fixture.reopen();
                // 开库就收口：提交了的散件删完，没提交的段清掉。
                assert_eq!(
                    loose_records(&fixture.data).len(),
                    if committed { 0 } else { before.len() },
                    "{point}"
                );
                assert_eq!(fixture.segments().len(), usize::from(committed), "{point}");
            }
            assert_eq!(all_records(&broker), before, "收口之后 {point}");
            for index in 0..6 {
                answers(&broker, index);
            }
            pack(&mut broker, later());
            assert!(loose_records(&fixture.data).is_empty(), "{point}");
            assert_eq!(all_records(&broker), before, "{point}");
            assert_eq!(fixture.state()["archive"]["packs"], 6, "{point}");
            let (_, sealed) = checkpoint(&fixture, broker, "seal");
            assert_eq!(sealed.unwrap(), 12, "{point}");
        }
    }
}

/// 还没收口就封存：提交了、散件没删完的照样成，重复的只算一条；段已落没提交的封不成。
#[test]
fn a_seal_before_settling_counts_each_record_once() {
    for point in [102, 103, 104] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 6);
        fault(&broker, point);
        assert!(
            broker
                .archive_step(later(), &mut ArchiveCursor::default())
                .is_err()
        );
        let (_, sealed) = checkpoint(&fixture, broker, "seal");
        assert_eq!(sealed.unwrap(), 12, "{point}");
    }
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    fault(&broker, 101);
    assert!(
        broker
            .archive_step(later(), &mut ArchiveCursor::default())
            .is_err()
    );
    let (mut broker, sealed) = checkpoint(&fixture, broker, "refused");
    assert!(sealed.is_err());
    // 下一步先收口，再封就成。
    assert_eq!(pack(&mut broker, later()), 6);
    let (_, sealed) = checkpoint(&fixture, broker, "sealed");
    assert_eq!(sealed.unwrap(), 12);
}

/// 收口只删逐字节相同的：孤儿段和散件对不上、没删完的散件和段对不上，都拒绝开库，什么都不删。
#[test]
fn settling_refuses_to_delete_what_does_not_match() {
    for point in [101, 102, 103] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 6);
        fault(&broker, point);
        assert!(
            broker
                .archive_step(later(), &mut ArchiveCursor::default())
                .is_err()
        );
        drop(broker);
        let replay = fs::read_dir(fixture.data.join("idempotency"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut bytes = fs::read(&replay).unwrap();
        bytes.push(b' ');
        fs::write(&replay, bytes).unwrap();
        let loose = loose_records(&fixture.data);
        let segments = fixture.segments();
        assert!(
            matches!(fixture.try_reopen(), Err(BrokerError::CorruptWriterState)),
            "{point}"
        );
        assert_eq!(loose_records(&fixture.data), loose, "{point}");
        assert_eq!(fixture.segments(), segments, "{point}");
    }
}

/// 步与步之间来了新写入：接着打，新的不受影响。
#[test]
fn writes_between_steps_are_fine() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 0..100);
    let mut cursor = ArchiveCursor::default();
    broker.archive_step(later(), &mut cursor).unwrap();
    broker.mutate_recorded(note(7), metadata()).unwrap();
    flush(&mut broker);
    while !broker.archive_step(later(), &mut cursor).unwrap().done {}
    pack(&mut broker, later());
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(fixture.state()["archive"]["packs"], 102);
    answers(&broker, 7);
}

/// 存状态的结果不明：写者拒绝新写，重开定论，不猜。
#[test]
fn an_unknown_state_save_is_settled_only_by_reopening() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 3);
    let before = all_records(&broker);
    broker.writer.poison();
    assert!(matches!(
        broker.archive_step(later(), &mut ArchiveCursor::default()),
        Err(BrokerError::MutationUnresolved)
    ));
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(all_records(&broker), before);
    assert_eq!(pack(&mut broker, later()), 3);
}
