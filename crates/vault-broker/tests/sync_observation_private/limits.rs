use super::*;

#[test]
fn per_rule_limit_and_whole_batch_limit_fail_before_first_body() {
    let f = Fixture::new();
    let mut broker = f.broker();
    f.write("notes/a", b"12");
    let mut narrow = selection();
    narrow.folders[0].max_bytes = 1;
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&narrow, &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(1), 0);
    for i in 0..17 {
        fs::File::create(f.root.join(format!("notes/large-{i}")))
            .unwrap()
            .set_len(256 * 1024 * 1024)
            .unwrap();
    }
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(1), 0);
}

#[test]
fn exact_256_mib_sparse_object_is_streamed_and_one_extra_byte_is_rejected() {
    let _serial = crate::blob::LARGE_IO.enter();
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    let target = f.root.join("notes/large");
    fs::File::create(&target)
        .unwrap()
        .set_len(256 * 1024 * 1024 + 1)
        .unwrap();
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(1), 0);
    fs::OpenOptions::new()
        .write(true)
        .open(&target)
        .unwrap()
        .set_len(256 * 1024 * 1024)
        .unwrap();
    reset();
    let proof = capture(&mut broker);
    assert_eq!(
        present(&proof, "notes/large").identity.bytes,
        256 * 1024 * 1024
    );
    assert!(present(&proof, "notes/large").utf8_valid);
    assert_eq!(count(3), 4096);
}

#[test]
fn object_and_required_path_caps_are_not_per_directory_budgets() {
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    let required: Vec<_> = (0..=budget)
        .map(|i| path(&format!("notes/missing-{i}")))
        .collect();
    assert!(
        broker
            .capture_sync_observation(&selection(), &required[..budget])
            .is_ok(),
        "恰好预算条必查路径必须通过"
    );
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &required),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(0), 0);
    for i in 0..budget {
        fs::File::create(f.root.join(format!("notes/file-{i}"))).unwrap();
    }
    assert!(
        broker.capture_sync_observation(&selection(), &[]).is_ok(),
        "恰好预算个文件必须通过"
    );
    fs::File::create(f.root.join("notes/file-extra")).unwrap();
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(1), 0);
}

/// BUG-047：169 第二次主库观察带「基线∪主库∪镜像」4108 条必查路径，旧的 4096 在这里报写者上限。
#[test]
fn union_of_4108_required_paths_seen_on_169_is_observed() {
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    for i in 0..1556 {
        fs::File::create(f.root.join(format!("notes/present-{i}"))).unwrap();
    }
    let required: Vec<_> = (0..4108)
        .map(|i| {
            if i < 1556 {
                path(&format!("notes/present-{i}"))
            } else {
                path(&format!("notes/remote-only-{i}"))
            }
        })
        .collect();
    let proof = broker
        .capture_sync_observation(&selection(), &required)
        .unwrap();
    assert_eq!(proof.entries().len(), 4108);
}

#[test]
fn excluded_entries_still_consume_global_enumeration_budget() {
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    for i in 0..65_536 {
        fs::File::create(f.root.join(format!("notes/.excluded-{i}"))).unwrap();
    }
    reset();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(1), 0);
}

#[test]
fn directory_depth_and_full_name_bytes_are_independently_bounded() {
    let f = Fixture::new();
    let mut broker = f.broker();
    fs::create_dir(f.root.join("notes")).unwrap();
    // 根、notes 与子目录合计恰好预算个目录可过，再多一个即拒绝。
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    for i in 0..budget - 2 {
        fs::create_dir(f.root.join(format!("notes/dir-{i}"))).unwrap();
    }
    assert!(broker.capture_sync_observation(&selection(), &[]).is_ok());
    fs::create_dir(f.root.join("notes/dir-extra")).unwrap();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    let deep = Fixture::new();
    let mut broker = deep.broker();
    let mut target = deep.root.join("notes");
    for _ in 0..63 {
        target = target.join("d");
    }
    fs::create_dir_all(&target).unwrap();
    assert!(broker.capture_sync_observation(&selection(), &[]).is_ok());
    fs::create_dir(target.join("too-deep")).unwrap();
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
    let names = Fixture::new();
    let mut broker = names.broker();
    fs::create_dir(names.root.join("notes")).unwrap();
    for i in 0..19_000 {
        fs::File::create(names.root.join(format!("notes/.{i}{}", "x".repeat(220)))).unwrap();
    }
    assert!(matches!(
        broker.capture_sync_observation(&selection(), &[]),
        Err(BrokerError::WriterLimitExceeded)
    ));
}
