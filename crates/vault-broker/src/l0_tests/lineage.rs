use super::*;

#[test]
fn lineage_follows_history_order_and_marks_only_a_recorded_creation() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (first, second) = versions(&mut broker);
    let lineage = broker.history_lineage(&path(), 50).unwrap();
    assert!(!lineage.more);
    let listed = broker.list_history(&path(), 50, None).unwrap().entries;
    assert_eq!(
        lineage
            .entries
            .iter()
            .map(|v| v.entry.clone())
            .collect::<Vec<_>>(),
        listed,
        "版本链和历史列表必须是同一份、同一个次序"
    );
    assert_eq!(
        lineage
            .entries
            .iter()
            .map(|v| (v.entry.revision, v.created))
            .collect::<Vec<_>>(),
        vec![(second, false), (first, true)],
        "只有装到不存在路径上的那一版算创建"
    );
}

#[test]
fn lineage_never_calls_pre_existing_bytes_a_creation() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    fs::write(fixture.root.join("note.md"), b"outside").unwrap();
    let old = broker.read(&path()).unwrap().revision;
    let new = broker
        .mutate_recorded(
            Mutation::replace(path(), b"saved".to_vec(), old, "saved"),
            metadata(),
        )
        .unwrap()
        .revision;
    let lineage = broker.history_lineage(&path(), 50).unwrap();
    assert_eq!(
        lineage
            .entries
            .iter()
            .map(|v| (v.entry.revision, v.created, v.entry.author.is_some()))
            .collect::<Vec<_>>(),
        vec![(new, false, true), (old, false, false)],
        "库里原来就有的正文作者未知，也不是任何人创建的"
    );
}

#[test]
fn lineage_is_bounded_and_says_when_older_versions_were_left_out() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (_, second) = versions(&mut broker);
    let head = broker.history_lineage(&path(), 1).unwrap();
    assert!(head.more);
    assert_eq!(head.entries.len(), 1);
    assert_eq!(head.entries[0].entry.revision, second);
    for limit in [0, 101] {
        assert!(matches!(
            broker.history_lineage(&path(), limit),
            Err(BrokerError::WriterLimitExceeded)
        ));
    }
    let untouched = broker
        .history_lineage(&VaultPath::parse("other.md").unwrap(), 50)
        .unwrap();
    assert!(untouched.entries.is_empty() && !untouched.more);
}
