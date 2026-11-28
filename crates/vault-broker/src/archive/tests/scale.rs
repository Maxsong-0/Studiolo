use super::*;
use std::time::Instant;

/// 满 4096 条散件时的现状：写、新建路径、封存都拒。打包之后全部恢复，旧键照答，封存逐条核过。
#[test]
fn six_thousand_records_keep_writing_answering_and_sealing() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 4);
    bulk(&broker, 0..4092);
    assert_eq!(broker.record_usage().unwrap().descriptors.used, 4096);
    assert!(matches!(
        broker.mutate_recorded(note(50), metadata()),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(pack(&mut broker, later()), 4096);
    broker.mutate_recorded(note(50), metadata()).unwrap();
    flush(&mut broker);
    bulk(&broker, 4092..6000);
    let started = Instant::now();
    assert_eq!(pack(&mut broker, later()), 1909);
    eprintln!("打 1909 包 {:?}", started.elapsed());
    broker.mutate_recorded(note(51), metadata()).unwrap();
    flush(&mut broker);
    let usage = broker.record_usage().unwrap();
    assert_eq!(
        (usage.packs, usage.descriptors.used, usage.replays.used),
        (6005, 1, 1)
    );
    for index in [0, 1, 2, 3, 50, 51] {
        answers(&broker, index);
    }
    for index in [0, 1, 777, 4091, 4092, 5999] {
        let request = bulk_request(index);
        assert_eq!(
            broker
                .mutation_replay_recorded(&request, &metadata())
                .unwrap()
                .unwrap()
                .path,
            request.path
        );
        let mut changed = request;
        changed.content.push(b'!');
        assert!(matches!(
            broker.mutation_replay_recorded(&changed, &metadata()),
            Err(BrokerError::IdempotencyConflict)
        ));
    }
    // 历史证据不断。
    for index in 0..4 {
        let path = logical(&format!("note-{index}.md"));
        assert!(
            !broker
                .list_history(&path, 10, None)
                .unwrap()
                .entries
                .is_empty()
        );
    }
    let started = Instant::now();
    let (broker, sealed) = checkpoint(&fixture, broker, "seal");
    eprintln!("封存 6006 条 {:?}", started.elapsed());
    assert_eq!(sealed.unwrap(), 2 * 6006);
    drop(broker);
    let started = Instant::now();
    drop(fixture.reopen());
    eprintln!("重开核 6005 包 {:?}", started.elapsed());
}

/// 重放可以先于描述符满：描述符没满、重放满了，封存、新建路径和文件操作一起拒；打包之后恢复。
#[test]
fn replays_that_fill_up_first_are_relieved_too() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    renames::enable(&mut broker);
    bulk(&broker, 0..4000);
    legacy_replays(&broker, 0..96);
    let usage = broker.record_usage().unwrap();
    assert_eq!((usage.descriptors.used, usage.replays.used), (4001, 4097));
    let (mut broker, sealed) = checkpoint(&fixture, broker, "full");
    assert!(matches!(sealed, Err(BrokerError::WriterLimitExceeded)));
    assert!(matches!(
        broker.list_history(&logical("brand-new.md"), 10, None),
        Err(BrokerError::WriterLimitExceeded)
    ));
    let move_it = renames::rename(&broker, "note-0.md", "moved.md", "move");
    assert!(matches!(
        broker.prepare_file_operation(move_it.clone(), renames::actor()),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(pack(&mut broker, later()), 4001);
    assert_eq!(broker.record_usage().unwrap().replays.used, 96);
    broker
        .list_history(&logical("brand-new.md"), 10, None)
        .unwrap();
    broker.mutate_recorded(note(60), metadata()).unwrap();
    flush(&mut broker);
    renames::run(&mut broker, move_it, 20_000);
    let (_, sealed) = checkpoint(&fixture, broker, "relieved");
    assert_eq!(sealed.unwrap(), 2 * 4003);
}

/// 正文写入不越过封存的上限（T-249 G1）：重放散件到了 4096 就先拒，封存照常成；打包腾出来以后照常写。
/// 原来正文写入只数描述符，写进去以后重放 4097 条，封存、新建路径、文件操作一起拒。
#[test]
fn a_content_write_never_pushes_replays_past_what_a_seal_accepts() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 0..3999);
    legacy_replays(&broker, 0..96);
    let usage = broker.record_usage().unwrap();
    assert_eq!((usage.descriptors.used, usage.replays.used), (4000, 4096));
    assert!(matches!(
        broker.mutate_recorded(note(70), metadata()),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(broker.record_usage().unwrap().replays.used, 4096);
    let (mut broker, sealed) = checkpoint(&fixture, broker, "full");
    assert_eq!(sealed.unwrap(), 2 * 4000);
    assert_eq!(pack(&mut broker, later()), 4000);
    broker.mutate_recorded(note(70), metadata()).unwrap();
    flush(&mut broker);
    let (_, sealed) = checkpoint(&fixture, broker, "relieved");
    assert_eq!(sealed.unwrap(), 2 * 4001);
}
