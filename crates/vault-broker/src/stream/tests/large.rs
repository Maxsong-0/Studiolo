use super::*;
#[test]
fn true_large_content_small_replacement_and_strict_restore_keep_exact_history() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.pattern(LARGE_BYTES);
    let request = request(&source, "large", Precondition::MustNotExist);
    assert_eq!(
        request.content.sha256,
        "1606f5a7d61b07583745b90864f05b332d60140b876c93025d07e1b1baaa13db"
    );
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    let large = b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(installed(&b).identity, request.content);
    // 原 Vec API 仍只接收小正文，但必须完整保全当前 200 MiB 旧对象。
    let small = b
        .mutate_recorded(
            Mutation::replace(path(), b"small".to_vec(), large.revision, "small"),
            metadata(),
        )
        .unwrap();
    let entries = b.list_history(&path(), 100, None).unwrap().entries;
    assert!(
        entries
            .iter()
            .any(|e| e.revision == large.revision && e.bytes == LARGE_BYTES as u64)
    );
    assert!(matches!(
        b.read_history(&path(), large.revision, 256 * 1024),
        Err(BrokerError::ReadLimitExceeded { .. })
    ));
    ack(&mut b);
    let ticket = b
        .prepare_stream_restore(
            path(),
            small.revision,
            large.revision,
            "restore".into(),
            actor(),
            limits(),
        )
        .unwrap();
    b.confirm_stream_restore_intent(&ticket, 1, &"b".repeat(64))
        .unwrap();
    let restored = b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(restored.revision, large.revision);
    assert_eq!(installed(&b).identity, request.content);
    assert!(b.content_audit_status().unwrap().restore_unconfirmed);
    ack(&mut b);
    assert!(!b.content_audit_status().unwrap().restore_unconfirmed);
    // 真正清理大旧对象，不能因旧 64 MiB GC 批预算而永久保留附件。
    let mut revision = restored.revision;
    for n in 0..52 {
        revision = b
            .mutate(Mutation::replace(
                path(),
                format!("v{n}").into_bytes(),
                revision,
                format!("v{n}"),
            ))
            .unwrap()
            .revision;
    }
    let catalog = f.data.join("history/meta-v1/catalogs").join(format!(
        "{}.json",
        crate::sha256(path().as_str().as_bytes())
    ));
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog).unwrap()).unwrap();
    for item in value["entries"].as_array_mut().unwrap() {
        item["entry"]["archived_at_ms"] = 1.into();
    }
    fs::write(catalog, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        b.prune_history(&path(), HistoryRetention::default(), 64)
            .unwrap()
            .removed_objects
            > 0
    );
    assert!(
        !f.data
            .join("history")
            .join(large.revision.to_string())
            .exists()
    );
    assert_eq!(
        b.restore_replay(&path(), small.revision, large.revision, "restore", &actor())
            .unwrap(),
        Some(restored)
    );
}
