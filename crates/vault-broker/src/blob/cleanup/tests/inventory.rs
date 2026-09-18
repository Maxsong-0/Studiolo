use super::*;

#[test]
fn last_temporary_after_a_full_producer_scan_can_still_be_cleaned() {
    let _guard = LARGE_IO.enter();
    let f = Fixture::new();
    let mut b = f.open();
    for index in 0..50_000 {
        fs::write(f.data.join("history").join(format!("{index:064x}")), b"").unwrap();
    }
    let residual = f.temporary("history", b"unfinished");
    assert_eq!(count(&f, "history"), 50_001);
    assert_eq!(
        b.prune_blob_temporaries().unwrap(),
        BlobPruneReport {
            removed: 1,
            bytes: 10
        }
    );
    assert!(!residual.exists());
    assert_eq!(count(&f, "history"), 50_000);
    assert!(
        f.data
            .join("history")
            .join(format!("{:064x}", 49_999))
            .exists()
    );
    // 第二个额外节点超出清理预算；错误不能隐式删掉本批的合法临时对象。
    let residual = f.temporary("history", b"retained");
    fs::write(f.data.join("history/extra-published"), b"keep").unwrap();
    assert!(matches!(
        b.prune_blob_temporaries(),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(fs::read(residual).unwrap(), b"retained");
    assert_eq!(count(&f, "history"), 50_002);
}

#[test]
fn namespace_collisions_are_rejected_before_a_candidate_can_be_unlinked() {
    // APFS 可能折叠大小写/规范形式；私有先验单独核对这种目录快照，不依赖主机布局。
    for names in [
        vec!["Published".to_owned(), "published".to_owned()],
        vec!["é".to_owned(), "e\u{301}".to_owned()],
    ] {
        assert!(matches!(
            scan::collision_check(&names),
            Err(BrokerError::PathCollision { .. })
        ));
    }
    let f = Fixture::new();
    let b = f.open();
    let residual = f.temporary("history", b"retained");
    let plan = prepare(&b).unwrap();
    fs::write(f.data.join("snapshots/new-unreviewed-entry"), b"added").unwrap();
    assert!(matches!(
        run::execute(&b, plan),
        Err(BrokerError::CaptureChanged)
    ));
    assert_eq!(fs::read(residual).unwrap(), b"retained");
}
