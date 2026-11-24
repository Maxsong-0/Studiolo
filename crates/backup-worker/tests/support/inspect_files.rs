use super::*;
use crate::seal_fixture::{Fixture, repo};
use std::cell::RefCell;

/// BUG-053：读回校验不能每个文件起一个 restic。169 上一次封存约 1.1 万个文件，每个 `restic dump --no-cache`
/// 都要从仓库重新加载索引、约 1 秒，核完要 3 小时，而封存副本 1 小时就到期，第一份备份永远核不完。
#[test]
fn the_whole_sealed_tree_is_read_back_by_one_restic_dump_of_its_root() {
    let f = Fixture::new();
    let seal = f.seal("read-back");
    let root = &seal.ticket().transfer_source.root;
    assert!(seal.inventory().files().count() >= 3);
    let snapshot = SnapshotRef::parse("ab".repeat(32)).unwrap();
    let repository = repo();
    let calls = RefCell::new(Vec::new());
    verify_files(
        &repository,
        &snapshot,
        root,
        seal.inventory(),
        &|argv, _| {
            calls.borrow_mut().push(argv.to_vec());
            Ok(())
        },
    )
    .unwrap();
    let expected: Vec<String> = vec![
        "--json".into(),
        "--no-cache".into(),
        "--repo".into(),
        repository.as_str().into(),
        "dump".into(),
        "--archive".into(),
        "tar".into(),
        snapshot.as_str().into(),
        root.to_str().unwrap().into(),
    ];
    assert_eq!(calls.into_inner(), vec![expected]);
}

/// 期望照清单造：盘上那份封存原样写成 tar 流能核过；少一个文件或改一个字节就不行。
#[test]
fn the_read_back_expects_exactly_the_sealed_inventory() {
    use crate::process::tar::builder;
    let f = Fixture::new();
    let seal = f.seal("read-back-content");
    let root = &seal.ticket().transfer_source.root;
    let snapshot = SnapshotRef::parse("ab".repeat(32)).unwrap();
    let name = |relative: &str| root.join(relative).to_str().unwrap()[1..].to_owned();
    let entries = |skip: Option<usize>, flip: Option<usize>| {
        let mut out = Vec::new();
        for directory in seal.inventory().directories() {
            out.extend(builder::directory(&name(directory)));
        }
        for (index, file) in seal.inventory().files().enumerate() {
            if skip == Some(index) {
                continue;
            }
            let mut body = std::fs::read(root.join(file.path().as_str())).unwrap();
            if flip == Some(index) {
                body[0] ^= 1;
            }
            out.extend(builder::file(&name(file.path().as_str()), &body));
        }
        out.extend(builder::end());
        out
    };
    let check = |stream: Vec<u8>| {
        verify_files(
            &repo(),
            &snapshot,
            root,
            seal.inventory(),
            &|_, expected| builder::check(expected, &stream, 4096),
        )
    };
    check(entries(None, None)).unwrap();
    assert_eq!(check(entries(Some(0), None)), Err(ProcessError::Protocol));
    assert_eq!(check(entries(None, Some(1))), Err(ProcessError::Protocol));
}
