//! 黄金值由独立 Python hashlib 依据既有 v1 字节协议产生，不调用被测实现。
use studiolo_core::{Revision, RevisionHasher, VaultPath, VaultUuid};
fn vault() -> VaultUuid {
    "11111111-2222-4333-8444-555555555555".parse().unwrap()
}
fn path() -> VaultPath {
    VaultPath::parse("附件/样本.bin").unwrap()
}

#[test]
fn independent_golden_vectors_keep_original_domain_and_encoding() {
    let cases = [
        (
            Vec::new(),
            "d690e019efbb402a1baa43b06160e73509362479c04f3ba0164bdd7acf033f0f",
        ),
        (
            (0..=255).collect(),
            "59a7a2632d960e57644fdb68be3ac1b8ac1489ddff4a346e69db992aed5c27c7",
        ),
        (
            "附件流式验证".as_bytes().to_vec(),
            "1a9925166fed03a0984452b3ec35e524bfaec921dad6876f904c64936ba70f83",
        ),
    ];
    for (bytes, expected) in cases {
        for width in [1, 3, 7, 64, 255, 256, 65_536] {
            let mut hasher = RevisionHasher::new(vault(), &path());
            for chunk in bytes.chunks(width) {
                hasher.update(chunk);
                hasher.update(&[]);
            }
            assert_eq!(hasher.finish().to_hex(), expected);
        }
        assert_eq!(
            Revision::compute(vault(), &path(), &bytes).to_hex(),
            expected
        );
    }
}
#[test]
fn arbitrary_binary_chunks_equal_the_existing_slice_api() {
    let bytes: Vec<_> = (0..131_101_u64)
        .map(|i| ((i * 131 + (i >> 16)) % 256) as u8)
        .collect();
    let expected = Revision::compute(vault(), &path(), &bytes);
    for width in [1, 31, 65_535, 65_536, 65_537, bytes.len()] {
        let mut hasher = RevisionHasher::new(vault(), &path());
        for chunk in bytes.chunks(width) {
            hasher.update(chunk);
        }
        assert_eq!(hasher.finish(), expected);
    }
}
#[test]
fn namespace_and_path_remain_part_of_identity() {
    let mut first = RevisionHasher::new(vault(), &path());
    first.update(b"same");
    let mut other_path = RevisionHasher::new(vault(), &VaultPath::parse("另一个.bin").unwrap());
    other_path.update(b"same");
    let mut other_vault = RevisionHasher::new(
        "21111111-2222-4333-8444-555555555555".parse().unwrap(),
        &path(),
    );
    other_vault.update(b"same");
    let expected = first.finish();
    assert_ne!(expected, other_path.finish());
    assert_ne!(expected, other_vault.finish());
}
