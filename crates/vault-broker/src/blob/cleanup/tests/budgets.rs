use super::*;

#[test]
fn each_directory_may_reach_its_producer_count_limit_and_cleanup_restores_capacity() {
    let f = Fixture::new();
    let mut b = f.open();
    for domain in ["history", "snapshots"] {
        for _ in 0..64 {
            f.temporary(domain, b"x");
        }
        let dir = directory(&b.data, domain).unwrap();
        assert!(matches!(
            crate::blob::quota::reserve_preservation(&dir, 1),
            Err(BrokerError::WriterLimitExceeded)
        ));
    }
    assert_eq!(
        b.prune_blob_temporaries().unwrap(),
        BlobPruneReport {
            removed: 128,
            bytes: 128
        }
    );
    for domain in ["history", "snapshots"] {
        assert_eq!(count(&f, domain), 0);
        crate::blob::quota::reserve_preservation(&directory(&b.data, domain).unwrap(), 1).unwrap();
    }
}

#[test]
fn full_512_mib_per_directory_is_legal_as_a_single_batch() {
    let _guard = LARGE_IO.enter();
    let f = Fixture::new();
    let mut b = f.open();
    for domain in ["history", "snapshots"] {
        f.sparse(domain, crate::blob::OBJECT_LIMIT);
        f.sparse(domain, crate::blob::OBJECT_LIMIT);
    }
    assert_eq!(
        b.prune_blob_temporaries().unwrap(),
        BlobPruneReport {
            removed: 4,
            bytes: 2 * crate::blob::STAGE_BYTES,
        }
    );
    for domain in ["history", "snapshots"] {
        crate::blob::quota::reserve_preservation(
            &directory(&b.data, domain).unwrap(),
            crate::blob::OBJECT_LIMIT,
        )
        .unwrap();
    }
}

#[test]
fn any_directory_over_count_object_or_total_budget_refuses_the_entire_batch() {
    let _guard = LARGE_IO.enter();
    for budget in [0, 1, 2] {
        let f = Fixture::new();
        let mut b = f.open();
        let safe = f.temporary("history", b"safe");
        match budget {
            0 => {
                for _ in 0..65 {
                    f.temporary("snapshots", b"");
                }
            }
            1 => {
                f.sparse("snapshots", crate::blob::OBJECT_LIMIT + 1);
            }
            _ => {
                f.sparse("snapshots", crate::blob::OBJECT_LIMIT);
                f.sparse("snapshots", crate::blob::OBJECT_LIMIT);
                f.temporary("snapshots", b"overflow");
            }
        }
        let before = count(&f, "snapshots");
        assert!(matches!(
            b.prune_blob_temporaries(),
            Err(BrokerError::WriterLimitExceeded)
        ));
        assert_eq!(fs::read(safe).unwrap(), b"safe");
        assert_eq!(count(&f, "snapshots"), before);
    }
}

#[test]
fn non_sparse_body_over_old_four_mib_limit_is_fully_verified_and_removed() {
    let _guard = LARGE_IO.enter();
    let f = Fixture::new();
    let mut b = f.open();
    let filename = f.temporary("history", b"");
    let mut output = fs::OpenOptions::new().write(true).open(&filename).unwrap();
    let chunk: Vec<u8> = (0..65536).map(|i| ((i * 19 + 7) % 251) as u8).collect();
    for _ in 0..80 {
        output.write_all(&chunk).unwrap();
    }
    output.write_all(&chunk[..17]).unwrap();
    output.sync_all().unwrap();
    drop(output);
    assert_eq!(
        b.prune_blob_temporaries().unwrap(),
        BlobPruneReport {
            removed: 1,
            bytes: 5 * 1024 * 1024 + 17
        }
    );
    assert!(!filename.exists());
}
