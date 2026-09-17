//! 双侧各有新文件时，通过真实首轮名字证明完整并集，不能在 DTO 外补 Missing。
use super::*;
use crate::{LocalReadPresence, ReadOnlyVault};
use std::collections::BTreeSet;

#[test]
fn two_real_roots_finish_the_same_union_without_rescan_or_body_reread() {
    let left = Fixture::new();
    let mut broker = left.broker();
    left.write("notes/left.md", b"left");
    left.write("notes/common.md", b"canonical");
    left.write("notes/.hidden", b"private");
    let right = Fixture::new();
    right.write("notes/new/deep/right.md", b"right");
    right.write("notes/common.md", &[0xff]);
    fs::create_dir(right.root.join("notes/empty")).unwrap();
    let reader = ReadOnlyVault::open(&right.root).unwrap();
    let mut rules = selection();
    rules.files.push(SyncReadRule {
        path: path("absent/exact.md"),
        max_bytes: 20,
    });
    let baseline = [path("notes/removed.md")];
    reset();
    let a = broker.begin_sync_observation(&rules, &baseline).unwrap();
    let b = reader.begin_scoped_observation(&rules, &baseline).unwrap();
    assert_eq!((count(1), count(3)), (0, 0));
    assert!(!a.paths().contains(&path("notes/.hidden")));
    assert!(!b.paths().contains(&path("notes/empty")));
    let union: BTreeSet<_> = a.paths().union(b.paths()).cloned().collect();
    assert_eq!(union.len(), 5);
    let required: Vec<_> = union.iter().cloned().collect();
    let enumerated = count(4);
    let a = broker.finish_sync_observation(a, &required).unwrap();
    let b = reader.finish_scoped_observation(b, &required).unwrap();
    assert_eq!(a.entries().keys().cloned().collect::<BTreeSet<_>>(), union);
    assert_eq!(b.entries().keys().cloned().collect::<BTreeSet<_>>(), union);
    assert_eq!(
        a.entries()[&path("notes/new/deep/right.md")],
        CanonicalSyncPresence::Missing
    );
    assert_eq!(
        b.entries()[&path("notes/left.md")],
        LocalReadPresence::Missing
    );
    for missing in ["notes/removed.md", "absent/exact.md"] {
        assert_eq!(a.entries()[&path(missing)], CanonicalSyncPresence::Missing);
        assert_eq!(b.entries()[&path(missing)], LocalReadPresence::Missing);
    }
    let LocalReadPresence::Present(raw) = &b.entries()[&path("notes/common.md")] else {
        panic!("完整读取应有 Present");
    };
    assert_eq!(raw.identity.bytes, 1);
    assert!(!raw.utf8_valid);
    assert_eq!(
        present(&a, "notes/left.md").revision,
        crate::Revision::compute(broker.vault_uuid, &path("notes/left.md"), b"left")
    );
    broker.revalidate_sync_observation(&a).unwrap();
    reader.revalidate_scoped_observation(&b).unwrap();
    assert_eq!(count(4), enumerated, "finish 不得重新枚举目录");
    assert_eq!((count(1), count(3)), (4, 4), "每个 Present 只完整读取一次");
    assert!(!right.data.exists());
    assert!(
        !right.root.join("absent").exists(),
        "精确 Missing 不得创建父目录"
    );
    assert_eq!(
        fs::read_dir(&right.root).unwrap().count(),
        1,
        "本地观察不得创建控制目录"
    );
}

#[test]
fn additional_names_reuse_original_excluded_and_hidden_collision_index() {
    let f = Fixture::new();
    f.write("notes/a", b"a");
    f.write("notes/.Straße", b"hidden");
    f.write("notes/generated.md", b"generated");
    let reader = ReadOnlyVault::open(&f.root).unwrap();
    for name in [
        "notes/.STRASSE",
        "notes/GENERATED.md",
        "notes/PRIVATE/child",
    ] {
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        reset();
        assert!(matches!(
            reader.finish_scoped_observation(survey, &[path(name)]),
            Err(BrokerError::PathCollision { .. })
        ));
        assert_eq!(count(1), 0);
    }
    for names in [
        vec![path("notes/generated.md")],
        vec![path("notes/TASKS.md")],
        vec![path("elsewhere/file")],
        vec![path("notes/new"), path("notes/new")],
        vec![path("notes")],
    ] {
        let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
        reset();
        assert!(reader.finish_scoped_observation(survey, &names).is_err());
        assert_eq!(count(1), 0);
    }
}

#[test]
fn original_selection_is_owned_and_additional_missing_is_bounded_after_union() {
    let f = Fixture::new();
    f.write("notes/a", b"a");
    let reader = ReadOnlyVault::open(&f.root).unwrap();
    let mut rules = selection();
    let survey = reader.begin_scoped_observation(&rules, &[]).unwrap();
    rules.excluded_prefixes.clear();
    assert!(
        reader
            .finish_scoped_observation(survey, &[path("notes/generated.md")])
            .is_err()
    );
    let original: Vec<_> = (0..2048)
        .map(|i| path(&format!("notes/missing-{i}")))
        .collect();
    let survey = reader
        .begin_scoped_observation(&selection(), &original)
        .unwrap();
    let additional: Vec<_> = (2048..4096)
        .map(|i| path(&format!("notes/missing-{i}")))
        .collect();
    reset();
    assert!(matches!(
        reader.finish_scoped_observation(survey, &additional),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(count(1), 0, "4096 个缺失加一个真实对象必须在首个正文前拒绝");
}

#[test]
fn local_utf8_facts_check_chunk_boundary_and_eof_without_canonical_namespace() {
    let f = Fixture::new();
    let mut bytes = vec![b'a'; 65535];
    bytes.extend_from_slice("😀中文".as_bytes());
    f.write("notes/complete", &bytes);
    f.write("notes/truncated", &[0xf0, 0x9f, 0x98]);
    f.write("notes/nul", &[0]);
    let reader = ReadOnlyVault::open(&f.root).unwrap();
    let survey = reader.begin_scoped_observation(&selection(), &[]).unwrap();
    let result = reader.finish_scoped_observation(survey, &[]).unwrap();
    for (name, valid) in [("complete", true), ("truncated", false), ("nul", true)] {
        let LocalReadPresence::Present(file) = &result.entries()[&path(&format!("notes/{name}"))]
        else {
            panic!("完整读取应有 Present");
        };
        assert_eq!(file.utf8_valid, valid);
    }
}
