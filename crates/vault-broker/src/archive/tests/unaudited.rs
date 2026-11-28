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
