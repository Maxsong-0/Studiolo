//! 没有审计主体的写入（看板投影那种）也能打包（T-249 G2）：没有回执，按最近一次成功封存的内容截止时刻判它已经进过备份。
use super::*;

fn board(index: usize) -> Mutation {
    Mutation::create(
        logical(&format!("board/{index}.md")),
        format!("任务 {index}").into_bytes(),
        format!("board-{index}"),
    )
}
/// 旧键答得出原回执，同一个键换了内容报冲突。
fn answers(broker: &FsVaultBroker, request: &Mutation) {
    assert_eq!(
        broker.mutation_replay(request).unwrap().unwrap().path,
        request.path
    );
    let mut changed = request.clone();
    changed.content.push(b'!');
    assert!(matches!(
        broker.mutation_replay(&changed),
        Err(BrokerError::IdempotencyConflict)
    ));
}

/// 满 4096 条没有主体的散件：原来永远不合格，写入一直被拒。打包之后照写、旧键照答、历史和封存照过。
#[test]
fn six_thousand_unaudited_writes_keep_writing_answering_and_sealing() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    for index in 0..3 {
        broker.mutate(board(index)).unwrap();
    }
    bulk_with(&broker, 0..4092, Shape::unaudited());
    assert_eq!(broker.record_usage().unwrap().descriptors.used, 4096);
    assert!(matches!(
        broker.mutate(board(10)),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(pack(&mut broker, later()), 4096);
    broker.mutate(board(10)).unwrap();
    bulk_with(&broker, 4092..6000, Shape::unaudited());
    assert_eq!(pack(&mut broker, later()), 1909);
    broker.mutate(board(11)).unwrap();
    let usage = broker.record_usage().unwrap();
    assert_eq!(
        (usage.packs, usage.descriptors.used, usage.replays.used),
        (6005, 1, 1)
    );
    for index in [0, 1, 2, 10, 11] {
        answers(&broker, &board(index));
    }
    for index in [0, 1, 4091, 4092, 5999] {
        answers(&broker, &unaudited_request(index));
    }
    for index in [0, 10] {
        assert!(
            !broker
                .list_history(&logical(&format!("board/{index}.md")), 10, None)
                .unwrap()
                .entries
                .is_empty()
        );
    }
    let (broker, sealed) = checkpoint(&fixture, broker, "seal");
    // 只有那一篇有主体的有回执可核。
    assert_eq!(sealed.unwrap(), 2);
    drop(broker);
    let broker = fixture.reopen();
    answers(&broker, &board(0));
}

/// 没有回执的只看时间：请求时间要早于成功封存的内容截止时刻（同一毫秒不算），还没有截止时刻就一条不打；
/// 有主体的那篇照旧看回执，水位是 0 就不打。
#[test]
fn an_unaudited_write_is_packed_only_once_a_seal_cut_off_after_it() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let shape = Shape::unaudited();
    bulk_with(&broker, 0..1, shape);
    for (sealed_content_ms, expected) in [
        (None, 0),
        (Some(i64::MIN), 0),
        (Some(shape.at), 0),
        (Some(shape.at + 1), 1),
    ] {
        let cutoff = ArchiveCutoff {
            sealed_offset: 0,
            sealed_content_ms,
            ..later()
        };
        assert_eq!(pack(&mut broker, cutoff), expected, "{sealed_content_ms:?}");
    }
    answers(&broker, &unaudited_request(0));
}

/// 没满 7 天的照旧不打，哪怕截止时刻早就过了；满 7 天当天起打。
#[test]
fn an_unaudited_write_must_also_be_seven_days_old() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let shape = Shape {
        at: now() - DAY,
        audited: false,
    };
    bulk_with(&broker, 0..1, shape);
    for (now_ms, expected) in [(shape.at + 7 * DAY - 1, 0), (shape.at + 7 * DAY, 1)] {
        let cutoff = ArchiveCutoff {
            sealed_offset: 0,
            sealed_content_ms: Some(shape.at + 1),
            now_ms,
        };
        assert_eq!(pack(&mut broker, cutoff), expected, "{now_ms}");
    }
}

/// 段里有了新类别，状态升第 10 版：说成第 9 版的不开（旧程序只认到 9，会把新类别的包当坏的）；
/// 只有老类别的照旧第 9 版。还原之后回到第 8 版、照常开库、旧键照答。
#[test]
fn a_new_kind_of_pack_raises_the_format_and_unpack_brings_it_back() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let audited_only = ArchiveCutoff {
        sealed_content_ms: None,
        ..later()
    };
    assert_eq!(pack(&mut broker, audited_only), 1);
    assert_eq!(fixture.state()["format_version"], 9);
    broker.mutate(board(0)).unwrap();
    assert_eq!(pack(&mut broker, later()), 1);
    assert_eq!(fixture.state()["format_version"], 10);
    drop(broker);
    let packed = fixture.state();
    let mut downgraded = packed.clone();
    downgraded["format_version"] = 9.into();
    fs::write(
        fixture.data.join("writer-state.json"),
        serde_json::to_vec(&downgraded).unwrap(),
    )
    .unwrap();
    assert!(fixture.try_reopen().is_err());
    fs::write(
        fixture.data.join("writer-state.json"),
        serde_json::to_vec(&packed).unwrap(),
    )
    .unwrap();
    let mut broker = fixture.reopen();
    let done = broker.archive_unpack().unwrap();
    assert_eq!((done.packs, done.descriptors, done.replays), (2, 2, 2));
    assert_eq!(fixture.state()["format_version"], 8);
    drop(broker);
    let broker = fixture.reopen();
    answers(&broker, &board(0));
}

/// 只有老类别的段，状态却说第 10 版：也不开（第 10 版当且仅当段里有新类别）。
#[test]
fn a_tenth_format_without_a_new_kind_of_pack_is_refused() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    assert_eq!(pack(&mut broker, later()), 1);
    drop(broker);
    let mut state = fixture.state();
    assert_eq!(state["format_version"], 9);
    state["format_version"] = 10.into();
    fs::write(
        fixture.data.join("writer-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert!(fixture.try_reopen().is_err());
}
