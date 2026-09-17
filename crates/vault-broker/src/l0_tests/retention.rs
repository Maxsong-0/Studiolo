use super::*;

fn catalog_file(fixture: &Fixture) -> PathBuf {
    fixture
        .data
        .join("history/meta-v1/catalogs")
        .join(format!("{}.json", sha256(path().as_str().as_bytes())))
}

#[test]
fn retention_requires_both_age_and_count_keeps_unknown_and_recovers_cleanup_interruptions() {
    for point in [15, 16] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let mut revisions = Vec::new();
        for n in 0..53 {
            let bytes = format!("version-{n}").into_bytes();
            let key = format!("key-{n}");
            let request = match revisions.last().copied() {
                Some(revision) => Mutation::replace(path(), bytes, revision, key),
                None => Mutation::create(path(), bytes, key),
            };
            revisions.push(broker.mutate(request).unwrap().revision);
        }
        assert_eq!(
            broker
                .prune_history(&path(), HistoryRetention::default(), 64)
                .unwrap()
                .removed_entries,
            0
        );
        let filename = catalog_file(&fixture);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&filename).unwrap()).unwrap();
        for item in value["entries"].as_array_mut().unwrap() {
            item["entry"]["archived_at_ms"] = 1.into();
        }
        value["entries"][0]["entry"]["archived_at_ms"] = serde_json::Value::Null;
        fs::write(&filename, serde_json::to_vec(&value).unwrap()).unwrap();
        interrupt(&broker, point);
        assert!(
            broker
                .prune_history(&path(), HistoryRetention::default(), 64)
                .is_err()
        );
        // 已退役的条目不能因实际删除尚未完成而重新被读取。
        assert!(matches!(
            broker.read_history(&path(), revisions[1], 100),
            Err(BrokerError::NotFound(_))
        ));
        drop(broker);
        let mut broker = fixture.reopen();
        broker
            .prune_history(&path(), HistoryRetention::default(), 64)
            .unwrap();
        assert_eq!(
            broker
                .list_history(&path(), 100, None)
                .unwrap()
                .entries
                .len(),
            51
        );
        assert!(
            fixture
                .data
                .join("history")
                .join(revisions[0].to_hex())
                .exists()
        );
        assert!(
            !fixture
                .data
                .join("history")
                .join(revisions[1].to_hex())
                .exists()
        );
        assert!(
            !fixture
                .data
                .join("history")
                .join(revisions[2].to_hex())
                .exists()
        );
        assert_eq!(
            broker
                .read_history(&path(), revisions[0], 100)
                .unwrap()
                .content,
            b"version-0"
        );
    }
}

#[test]
fn garbage_preflight_verifies_all_candidates_before_removing_any_object() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (old, _) = versions(&mut broker);
    let filename = catalog_file(&fixture);
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&filename).unwrap()).unwrap();
    value["entries"].as_array_mut().unwrap().remove(0);
    value["garbage"] = serde_json::to_value(vec![old]).unwrap();
    fs::write(&filename, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(fixture.data.join("history").join(old.to_hex()), b"corrupt").unwrap();
    assert!(matches!(
        broker.prune_history(&path(), HistoryRetention::default(), 64),
        Err(BrokerError::CorruptWriterState)
    ));
    assert_eq!(
        fs::read(fixture.data.join("history").join(old.to_hex())).unwrap(),
        b"corrupt"
    );
    assert_eq!(fixture.note(), b"B");
}

#[test]
fn public_history_and_delivery_budgets_refuse_unbounded_requests() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    broker
        .mutate_recorded(request(b"A", "first"), metadata())
        .unwrap();
    for limit in [0, 101, usize::MAX] {
        assert!(matches!(
            broker.list_history(&path(), limit, None),
            Err(BrokerError::WriterLimitExceeded)
        ));
    }
    for limit in [0, 65, usize::MAX] {
        assert!(broker.pending_content_audit(limit).is_err());
        assert!(broker.pending_recorded_restores(limit).is_err());
    }
    assert!(
        broker
            .prune_history(
                &path(),
                HistoryRetention {
                    versions: 49,
                    days: 30
                },
                64
            )
            .is_err()
    );
    assert!(
        broker
            .prune_history(
                &path(),
                HistoryRetention {
                    versions: 50,
                    days: 29
                },
                64
            )
            .is_err()
    );
    assert!(
        broker
            .prune_history(&path(), HistoryRetention::default(), 65)
            .is_err()
    );
}
