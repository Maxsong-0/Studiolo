use super::*;

/// 每个断点：断开、（进程死了）重开，记录一条不丢、一条不重，接着打完。
/// 100 段还没写；101 段已落、状态没存（B）；102 状态已存、散件没删（C）；103 一包删了两条、104 删过半（D）；
/// 105 删完没落盘（E）。段写到一半（A）和 E 之后散件重现，在下面各自的用例里；F、G 是还原的，在 `unpacked`。
#[test]
fn every_crash_point_resumes_without_losing_or_repeating_a_record() {
    for point in 100..=105 {
        for reopened in [true, false] {
            let fixture = Fixture::new();
            let mut broker = fixture.open();
            notes(&mut broker, 6);
            let before = all_records(&broker);
            fault(&broker, point);
            interrupted(&mut broker);
            assert_eq!(all_records(&broker), before, "断开那一刻 {point}");
            let committed = point >= 102;
            assert_eq!(fixture.state()["archive"].is_object(), committed, "{point}");
            // 真断在说的那一步：盘上几段、几条散件。
            assert_eq!(
                (fixture.segments().len(), loose_records(&fixture.data).len()),
                match point {
                    100 => (0, 24),
                    101 | 102 => (1, 24),
                    103 => (1, 22),
                    104 => (1, 11),
                    _ => (1, 0),
                },
                "{point}"
            );
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
        interrupted(&mut broker);
        let (_, sealed) = checkpoint(&fixture, broker, "seal");
        assert_eq!(sealed.unwrap(), 12, "{point}");
    }
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    fault(&broker, 101);
    interrupted(&mut broker);
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
        interrupted(&mut broker);
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
    let mut step = |broker: &mut FsVaultBroker| {
        broker
            .archive_step(later(), ArchiveBatch::All, &mut cursor)
            .unwrap()
    };
    // 挑好、还没写的时候来一条；写完一段、还没接着挑的时候再来一条。
    assert_eq!(step(&mut broker).packed, 0);
    broker.mutate_recorded(note(7), metadata()).unwrap();
    flush(&mut broker);
    assert_eq!(step(&mut broker).packed, 64);
    broker.mutate_recorded(note(8), metadata()).unwrap();
    flush(&mut broker);
    while !step(&mut broker).done {}
    pack(&mut broker, later());
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(fixture.state()["archive"]["packs"], 103);
    answers(&broker, 7);
    answers(&broker, 8);
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
        interrupted(&mut broker),
        BrokerError::MutationUnresolved
    ));
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(all_records(&broker), before);
    assert_eq!(pack(&mut broker, later()), 3);
}

/// 状态已经落盘、存的时候却报了错：写者不知道提交了没有。走一步、单独收口都拒绝，什么都不删
/// （这时内存里的链是旧的，照它收口会把已经提交的段当孤儿删掉）；重开定论：提交了，接着删散件。
#[test]
fn a_state_that_landed_but_reported_an_error_is_settled_only_by_reopening() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 3);
    let before = all_records(&broker);
    fault(&broker, 110);
    assert!(matches!(interrupted(&mut broker), BrokerError::Io { .. }));
    assert_eq!(fixture.state()["archive"]["packs"], 3);
    let on_disk = (loose_records(&fixture.data), fixture.segments());
    assert_eq!((on_disk.0.len(), on_disk.1.len()), (12, 1));
    for _ in 0..2 {
        assert!(matches!(
            interrupted(&mut broker),
            BrokerError::MutationUnresolved
        ));
        assert!(matches!(
            broker.archive_settle(),
            Err(BrokerError::MutationUnresolved)
        ));
    }
    assert!(broker.mutate_recorded(note(7), metadata()).is_err());
    assert_eq!((loose_records(&fixture.data), fixture.segments()), on_disk);
    assert_eq!(all_records(&broker), before);
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(fixture.segments(), on_disk.1);
    assert_eq!(all_records(&broker), before);
    for index in 0..3 {
        answers(&broker, index);
    }
    assert!(!broker.archive_settle().unwrap());
    assert_eq!(pack(&mut broker, later()), 0);
    let (_, sealed) = checkpoint(&fixture, broker, "seal");
    assert_eq!(sealed.unwrap(), 6);
}

/// 崩溃点 A：段文件的临时文件写完、还没改名，进程就死了。盘上真留着那个临时文件，没有段。
fn dies_writing_a_segment(broker: &mut FsVaultBroker, fixture: &Fixture) -> PathBuf {
    fault(broker, 109);
    let died = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| interrupted(broker)));
    assert!(died.is_err());
    let left = residues(fixture);
    assert_eq!((left.len(), fixture.segments().len()), (1, 1));
    assert!(fixture.state().get("archive").is_none());
    left.into_iter().next().unwrap()
}

/// A，重开：开库清扫把残留挪进隔离区（写全的、只写了一半的都一样），散件一条不少，接着打、封存都成。
#[test]
fn a_segment_left_half_written_is_swept_away_when_reopened() {
    for half in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 6);
        let before = all_records(&broker);
        let left = dies_writing_a_segment(&mut broker, &fixture);
        if half {
            let bytes = fs::read(&left).unwrap();
            fs::write(&left, &bytes[..bytes.len() / 2]).unwrap();
        }
        drop(broker);
        let mut broker = fixture.reopen();
        assert!(!left.exists());
        let name = left.file_name().unwrap();
        let kept = fs::read_dir(fixture.data.join("residue-quarantine"))
            .unwrap()
            .map(|batch| batch.unwrap().path().join("archive-v1").join(name))
            .filter(|path| path.exists())
            .count();
        assert_eq!(kept, 1);
        assert_eq!(all_records(&broker), before);
        assert_eq!(loose_records(&fixture.data).len(), before.len());
        assert_eq!(pack(&mut broker, later()), 6);
        assert_eq!(all_records(&broker), before);
        let (_, sealed) = checkpoint(&fixture, broker, "seal");
        assert_eq!(sealed.unwrap(), 12);
    }
}

/// 真封存的最后一关：导出物逐个归类，封存前拒绝归不了类的。
fn exported_by_a_seal(
    fixture: &Fixture,
    broker: FsVaultBroker,
    name: &str,
) -> (FsVaultBroker, Result<usize, BrokerError>) {
    let vault = broker.vault_uuid;
    let (broker, sealed) = checkpoint(fixture, broker, name);
    let export = fixture.target(&format!("{name}-export"));
    for (from, to) in [("vault", "canonical"), ("data", "writer-data")] {
        fs::rename(
            fixture.temp.path().join(format!("{name}-{from}")),
            export.join(to),
        )
        .unwrap();
    }
    let classified = sealed.and_then(|_| read_trusted_export(&export, vault));
    (broker, classified.map(|facts| facts.len()))
}

/// A，残留没挪走（还没重开）：它不是段，读、接着打都不受影响；封存拒绝它，不会把它当成记录带走。重开之后封得成。
#[test]
fn a_residue_that_was_not_swept_is_no_segment_and_blocks_the_seal() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    let before = all_records(&broker);
    let left = dies_writing_a_segment(&mut broker, &fixture);
    assert_eq!(all_records(&broker), before);
    let (mut broker, sealed) = exported_by_a_seal(&fixture, broker, "refused");
    assert!(sealed.is_err());
    // 收口不动它，也不因为它拒绝。
    broker.archive_settle().unwrap();
    assert!(left.exists());
    assert_eq!(pack(&mut broker, later()), 6);
    assert_eq!(all_records(&broker), before);
    assert_eq!(fixture.state()["archive"]["packs"], 6);
    let (broker, sealed) = exported_by_a_seal(&fixture, broker, "refused-again");
    assert!(sealed.is_err());
    drop(broker);
    let broker = fixture.reopen();
    assert!(!left.exists());
    assert_eq!(all_records(&broker), before);
    let (_, sealed) = exported_by_a_seal(&fixture, broker, "sealed");
    assert!(sealed.unwrap() > 0);
}

/// 崩溃点 E：散件删完、目录没落盘就断电，删掉的重现。重开逐字节核同再删，只算一条。
#[test]
fn records_that_reappear_after_the_delete_are_removed_again() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    let before = all_records(&broker);
    let loose = loose_records(&fixture.data);
    fault(&broker, 105);
    interrupted(&mut broker);
    assert!(loose_records(&fixture.data).is_empty());
    for ((path, name), bytes) in loose.iter().step_by(5) {
        fs::write(fixture.data.join(path).join(name), bytes).unwrap();
    }
    drop(broker);
    let broker = fixture.reopen();
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(all_records(&broker), before);
    let (_, sealed) = checkpoint(&fixture, broker, "seal");
    assert_eq!(sealed.unwrap(), 12);
}

/// 一步失败留下孤儿段，写者没死：单独收口一次就清掉，不用等下一次打包；封存跟着做得成。
#[test]
fn settling_alone_clears_an_uncommitted_segment_without_reopening() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    assert!(!broker.archive_settle().unwrap());
    fault(&broker, 101);
    interrupted(&mut broker);
    let (mut broker, sealed) = checkpoint(&fixture, broker, "refused");
    assert!(sealed.is_err());
    // 有在途的事先不收，什么都不动。
    broker.mutate_recorded(note(7), metadata()).unwrap();
    assert!(matches!(
        broker.archive_settle(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(fixture.segments().len(), 1);
    flush(&mut broker);
    assert!(broker.archive_settle().unwrap());
    assert!(!fixture.data.join("archive-v1").exists());
    assert!(!broker.archive_settle().unwrap());
    let (_, sealed) = checkpoint(&fixture, broker, "sealed");
    assert_eq!(sealed.unwrap(), 14);
}

/// 目录建好、段还没写就出错：已经动过盘，单独收口把空目录清掉。
#[test]
fn settling_alone_removes_the_empty_directory_of_a_step_that_failed_early() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    fault(&broker, 100);
    interrupted(&mut broker);
    assert!(fixture.data.join("archive-v1").exists());
    assert!(broker.archive_settle().unwrap());
    assert!(!fixture.data.join("archive-v1").exists());
    assert!(!broker.archive_settle().unwrap());
    assert_eq!(pack(&mut broker, later()), 6);
    assert!(!broker.archive_settle().unwrap());
}

/// 提交了、散件没删完的，也收得掉。
#[test]
fn settling_alone_finishes_a_committed_step() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 6);
    let before = all_records(&broker);
    fault(&broker, 103);
    interrupted(&mut broker);
    assert!(broker.archive_settle().unwrap());
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(all_records(&broker), before);
}
