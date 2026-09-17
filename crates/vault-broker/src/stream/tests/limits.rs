use super::*;
fn orphan(f: &Fixture, digit: char, size: u64) {
    // 仅配额计费测试使用稀疏文件；大正文正确性测试始终逐块写满。
    let file = fs::File::create(
        f.root
            .join(".workbench/staging")
            .join(format!("{}.stage", digit.to_string().repeat(64))),
    )
    .unwrap();
    file.set_len(size).unwrap();
}
#[test]
fn source_growth_near_total_limit_never_writes_more_than_the_reservation() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"one-byte");
    let request = request(&source, "grow", Precondition::MustNotExist);
    orphan(&f, 'a', crate::blob::OBJECT_LIMIT);
    orphan(&f, 'b', crate::blob::OBJECT_LIMIT - request.content.bytes);
    fs::write(f.source.join("input.bin"), vec![7u8; 1024 * 1024]).unwrap();
    // 重新选择已增长的稳定来源，排除 open 节点变化检查提前拒绝而掩盖预算回归。
    let source = f.input();
    assert!(
        b.prepare_stream_mutation(request.clone(), &source, limits(), metadata())
            .is_err()
    );
    let sizes: Vec<_> = stage_files(&f)
        .iter()
        .map(|p| fs::metadata(p).unwrap().len())
        .collect();
    assert_eq!(sizes.len(), 3);
    assert!(sizes.iter().copied().sum::<u64>() <= crate::blob::STAGE_BYTES);
    assert!(
        sizes
            .iter()
            .filter(|size| **size <= request.content.bytes)
            .count()
            >= 1
    );
    assert!(!f.root.join(path().as_str()).exists());
}
#[test]
fn over_total_and_count_fail_before_creating_another_stage() {
    let _guard = LARGE.lock().unwrap();
    for count in [false, true] {
        let f = Fixture::new();
        let mut b = f.open();
        let source = f.bytes(b"one");
        if count {
            for n in 0..64 {
                fs::write(
                    f.root
                        .join(".workbench/staging")
                        .join(format!("{n:064x}.stage")),
                    [],
                )
                .unwrap();
            }
        } else {
            orphan(&f, 'a', crate::blob::OBJECT_LIMIT);
            orphan(&f, 'b', crate::blob::OBJECT_LIMIT);
        }
        let before = stage_files(&f).len();
        assert!(matches!(
            b.prepare_stream_mutation(
                request(&source, "new", Precondition::MustNotExist),
                &source,
                limits(),
                metadata()
            ),
            Err(BrokerError::WriterLimitExceeded)
        ));
        assert_eq!(stage_files(&f).len(), before);
    }
}
#[test]
fn tampered_hash_and_effective_limit_refuse_atomic_prepare() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"hello");
    let mut request = request(&source, "new", Precondition::MustNotExist);
    assert!(
        b.prepare_stream_mutation(
            request.clone(),
            &source,
            StreamLimits::new(4).unwrap(),
            metadata()
        )
        .is_err()
    );
    assert!(stage_files(&f).is_empty());
    request.content.sha256 = "a".repeat(64);
    assert!(
        b.prepare_stream_mutation(request, &source, limits(), metadata())
            .is_err()
    );
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Ready);
    assert!(!f.root.join(path().as_str()).exists());
}
#[test]
fn all_orphan_bytes_are_validated_before_pruning() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    orphan(&f, 'a', 5 * 1024 * 1024);
    fs::write(f.root.join(".workbench/staging/unknown.stage"), b"bad").unwrap();
    assert!(b.prune_stages().is_err());
    assert_eq!(stage_files(&f).len(), 2);
    fs::remove_file(f.root.join(".workbench/staging/unknown.stage")).unwrap();
    assert_eq!(b.prune_stages().unwrap().removed, 1);
}
