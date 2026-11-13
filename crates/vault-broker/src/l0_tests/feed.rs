use super::*;
#[test]
fn feed_is_cross_path_bound_and_pagination_excludes_later_commits() {
    let f = Fixture::new();
    let mut b = f.open();
    for i in 0..35 {
        b.mutate_recorded(
            Mutation::create(
                VaultPath::parse(format!("notes/{i}.md")).unwrap(),
                format!("body-{i}").into_bytes(),
                format!("id-{i}"),
            ),
            metadata(),
        )
        .unwrap();
    }
    let first = b.list_changes(3, None).unwrap();
    assert_eq!(first.entries.len(), 3);
    assert_eq!(first.through_sequence, 35);
    b.mutate_recorded(
        Mutation::create(
            VaultPath::parse("later.md").unwrap(),
            b"later".to_vec(),
            "later",
        ),
        metadata(),
    )
    .unwrap();
    let mut entries = first.entries;
    let mut cursor = first.next_cursor;
    while let Some(value) = cursor {
        let page = b.list_changes(3, Some(&value)).unwrap();
        assert_eq!(page.through_sequence, 35);
        assert!(!page.history_incomplete);
        entries.extend(page.entries);
        cursor = page.next_cursor;
    }
    assert_eq!(entries.len(), 35);
    assert_eq!(
        entries.iter().map(|r| r.sequence).collect::<Vec<_>>(),
        (1..=35).rev().collect::<Vec<_>>()
    );
    assert!(entries.iter().all(|r| r.kind == ChangeKind::Created
        && r.actor == Some(actor())
        && r.previous_revision.is_none()
        && r.revision.is_some()
        && r.at_ms.is_some()));
    for cursor in [
        "f1:999:0:0",
        "f1:35:999:0",
        "f1:35:0:513",
        "f1:035:0:0",
        "f1:35:0:-1",
    ] {
        assert!(matches!(
            b.list_changes(10, Some(cursor)),
            Err(BrokerError::InvalidHistoryCursor)
        ));
    }
}
#[test]
fn feed_completion_is_idempotent_after_index_durable_before_watermark() {
    let f = Fixture::new();
    let mut b = f.open();
    interrupt(&b, 90);
    assert!(
        b.mutate_recorded(request(b"first", "interrupted"), metadata())
            .is_err()
    );
    assert!(matches!(
        b.list_changes(10, None),
        Err(BrokerError::MutationUnresolved)
    ));
    drop(b);
    let mut b = f.reopen();
    b.mutate_recorded(request(b"first", "interrupted"), metadata())
        .unwrap();
    assert_eq!(b.list_changes(10, None).unwrap().entries.len(), 1);
}
#[test]
fn feed_tampering_is_rejected_and_missing_buckets_are_reported_as_gaps() {
    let f = Fixture::new();
    let mut b = f.open();
    b.mutate_recorded(request(b"body", "key"), metadata())
        .unwrap();
    let filename = f.data.join("history/meta-v1/catalogs/feed-v1-0.json");
    let original = fs::read(&filename).unwrap();
    for field in ["path", "id", "sequence"] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["rows"][0][field] = if field == "sequence" {
            2.into()
        } else if field == "path" {
            "wrong.md".into()
        } else {
            "a".repeat(64).into()
        };
        fs::write(&filename, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            b.list_changes(10, None),
            Err(BrokerError::CorruptWriterState)
        ));
    }
    fs::remove_file(&filename).unwrap();
    let page = b.list_changes(10, None).unwrap();
    assert!(page.entries.is_empty());
    assert!(page.history_incomplete);
}
#[test]
fn retired_feed_entry_does_not_reappear_after_prune_interruption() {
    let f = Fixture::new();
    let mut b = f.open();
    let mut revision = None;
    for i in 0..53 {
        let m = match revision {
            Some(r) => {
                Mutation::replace(path(), format!("v{i}").into_bytes(), r, format!("key{i}"))
            }
            None => request(b"v0", "key0"),
        };
        revision = Some(b.mutate_recorded(m, metadata()).unwrap().revision);
    }
    let file = f
        .data
        .join("history/meta-v1/catalogs")
        .join(format!("{}.json", sha256(path().as_str().as_bytes())));
    let mut c: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    for row in c["entries"].as_array_mut().unwrap() {
        row["entry"]["archived_at_ms"] = 1.into();
    }
    fs::write(file, serde_json::to_vec(&c).unwrap()).unwrap();
    interrupt(&b, 15);
    assert!(
        b.prune_history(&path(), HistoryRetention::default(), 64)
            .is_err()
    );
    let mut cursor = None;
    let mut count = 0;
    let mut retired = false;
    loop {
        let page = b.list_changes(100, cursor.as_deref()).unwrap();
        count += page.entries.len();
        retired |= page.retired;
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(count, 50);
    assert!(retired);
    drop(b);
    let mut b = f.reopen();
    b.prune_history(&path(), HistoryRetention::default(), 64)
        .unwrap();
    let page = b.list_changes(100, Some("f1:53:0:0")).unwrap();
    assert!(page.retired);
    assert_eq!(page.entries.len(), 13);
}

#[test]
fn memory_and_disk_plain_writes_have_the_same_feed_shape() {
    macro_rules! check {
        ($broker:expr) => {{
            let mut broker = $broker;
            let first = broker.mutate(request(b"old", "plain-1")).unwrap();
            broker
                .mutate(Mutation::replace(
                    path(),
                    b"new".to_vec(),
                    first.revision,
                    "plain-2",
                ))
                .unwrap();
            let rows = broker.list_changes(100, None).unwrap();
            assert_eq!(rows.entries.len(), 2);
            assert_eq!(rows.through_sequence, 2);
            assert_eq!(rows.entries[0].kind, ChangeKind::Replaced);
            assert_eq!(rows.entries[0].actor, None);
            assert_eq!(rows.entries[0].previous_revision, Some(first.revision));
            assert_eq!(rows.entries[0].bytes, 3);
            assert_eq!(rows.entries[1].kind, ChangeKind::Created);
            assert_eq!(
                broker
                    .read_history(&path(), first.revision, 100)
                    .unwrap()
                    .content,
                b"old"
            );
            assert_eq!(
                broker
                    .list_history(&path(), 100, None)
                    .unwrap()
                    .entries
                    .len(),
                2
            );
        }};
    }
    check!(MemoryVaultBroker::new());
    let fixture = Fixture::new();
    check!(fixture.open());
}
