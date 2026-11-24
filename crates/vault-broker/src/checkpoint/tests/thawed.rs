//! 定格之后解冻写者（BUG-045）：写者照常改、换文件，导出的仍是冻结那一刻；
//! 有人原地改了定格的文件就整份失败、目标不动；能同时开着的文件数不够就定格失败，不半拍。
use super::*;

#[test]
fn capture_exports_the_frozen_moment_after_the_writer_thaws_and_rewrites() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    let first = broker.read(&path()).unwrap().revision;
    let mut frozen = broker.freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    let watermark = frozen.watermark().unwrap();
    let capture = frozen.take_capture().unwrap();
    assert!(frozen.take_capture().is_err());
    let mut broker = frozen.thaw();
    // 解冻之后写者照常改：新建再改名换上，冻结时那个 inode 只剩定格的句柄指着。
    broker
        .mutate_recorded(
            Mutation::replace(path(), "改过".as_bytes().to_vec(), first, "second"),
            MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap(),
        )
        .unwrap();
    assert_eq!(broker.read(&path()).unwrap().content, "改过".as_bytes());
    let vault = fixture.target("copy-vault");
    let data = fixture.target("copy-data");
    // 回执也核的是冻结那一刻：解冻之后那一笔还没落审计，活的写者这时核不过，副本照样核得过。
    let (sealed, coverage) = capture
        .export(&vault, &data, deadline())
        .unwrap()
        .verify(deadline(), verify)
        .unwrap();
    assert_eq!((coverage.checked(), coverage.max_offset()), (2, 2));
    assert!(verify_live(&broker, verify).is_err());
    assert!(sealed.is_verified());
    assert_eq!(sealed.watermark, watermark);
    assert_eq!(fs::read(vault.join("note.md")).unwrap(), "正文".as_bytes());
    // 写者数据也是冻结那一刻：换回原根重绑定后打开，读到的是旧内容。
    let OwnerWatermark::Broker {
        vault_uuid,
        pair_id,
        ..
    } = sealed.watermark
    else {
        panic!("wrong owner");
    };
    crate::rebind_new_roots(&crate::NewRootRebind {
        new_vault_root: &vault,
        new_data_root: &data,
        expected_vault_uuid: vault_uuid,
        expected_pair_id: pair_id,
        checkpoint_id: "checkpoint-after-thaw".into(),
        restore_id: "restore-after-thaw",
        parent_lineage: None,
        resume_interrupted: false,
    })
    .unwrap();
    drop(broker);
    let copy = FsVaultBroker::open_existing(&vault, &data).unwrap();
    let read = copy.read(&path()).unwrap();
    assert_eq!(read.content, "正文".as_bytes());
    assert_eq!(read.revision, first);
}

#[test]
fn capture_rejects_a_file_changed_in_place_after_thaw_without_touching_the_targets() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    let mut frozen = broker.freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    let capture = frozen.take_capture().unwrap();
    let broker = frozen.thaw();
    // 写者从不原地改；这里绕过写者直接改同一个 inode（长度、修改时间都变了）。
    fs::write(fixture.vault.join("note.md"), b"changed").unwrap();
    let vault = fixture.target("copy-vault");
    let data = fixture.target("copy-data");
    assert!(matches!(
        capture.export(&vault, &data, deadline()),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(fs::read_dir(&vault).unwrap().next().is_none());
    assert!(fs::read_dir(&data).unwrap().next().is_none());
    drop(broker);
}

#[test]
fn capture_fails_closed_when_the_open_file_budget_is_too_small() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    seed(&mut broker);
    flush(&mut broker);
    let root =
        cap_std::fs::Dir::open_ambient_dir(&fixture.vault, cap_std::ambient_authority()).unwrap();
    assert!(matches!(
        capture::capture(
            &root,
            VAULT_QUARANTINE,
            &mut capture::Budget::with_limit(0),
            deadline()
        ),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(
        capture::capture(
            &root,
            VAULT_QUARANTINE,
            &mut capture::Budget::with_limit(1_000),
            deadline()
        )
        .is_ok()
    );
    // 本进程的上限是软上限的一半；用例进程的软上限至少够开这一个文件。
    assert!(
        capture::capture(
            &root,
            VAULT_QUARANTINE,
            &mut capture::Budget::from_process(),
            deadline()
        )
        .is_ok()
    );
    drop(broker);
}

/// 拷大文件时没落盘的量封顶（BUG-054）：每写满 64 MiB 落一次盘，不等写完；一个文件一份计数。
#[test]
fn copying_a_large_file_flushes_every_64_mib_while_it_is_written() {
    const MIB: u64 = 1 << 20;
    let fixture = Fixture::new();
    let source = fixture.target("large");
    // 空洞文件：读出来是 0，拷的时候照样一个字节一个字节写进副本。
    // 前两个加起来正好一段：计数要是跨文件接着数，第二个就会落一次。
    for (name, mebibytes) in [("a-one", 1), ("b-short", 63), ("c-large", 130)] {
        let file = fs::File::create(source.join(name)).unwrap();
        file.set_len(mebibytes * MIB).unwrap();
    }
    let open = |path: &PathBuf| {
        cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
    };
    let captured = capture::capture(
        &open(&source),
        &[],
        &mut capture::Budget::with_limit(1_000),
        deadline(),
    )
    .unwrap();
    let copy = fixture.target("copy");
    capture::FLUSHED.take();
    captured
        .copy(
            &open(&copy),
            &mut super::super::exported::Originals::default(),
            deadline(),
        )
        .unwrap();
    assert_eq!(
        capture::FLUSHED.take(),
        [64, 128].map(|at| at * MIB),
        "130 MiB 的落两次，其余的写完才落"
    );
    assert_eq!(fs::metadata(copy.join("c-large")).unwrap().len(), 130 * MIB);
}
