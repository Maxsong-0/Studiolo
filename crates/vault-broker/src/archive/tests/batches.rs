//! 一段装多少：段数有上限、段又不能合并，所以段数要按包数涨，不能按轮数、步数涨。
use super::*;

/// 陆续到期：每轮只有几条新到水位之内。段数按包数涨，最后只留凑不够半段的几条。
#[test]
fn records_that_come_due_a_few_at_a_time_do_not_use_up_the_segments() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 0..320);
    let mut packed = 0;
    for due in (4..=320).step_by(4) {
        let cutoff = ArchiveCutoff {
            sealed_offset: 2 * due,
            ..later()
        };
        packed += round(&mut broker, cutoff, ArchiveBatch::Usual).unwrap();
        let segments = fixture.segments().len();
        assert!(
            segments * 32 <= packed,
            "第 {due} 条到期：{segments} 段 {packed} 包"
        );
    }
    assert!((289..=321).contains(&packed), "{packed}");
}

/// 合格的零散夹在不合格的中间，一步只凑得出几条：跨步留着，凑够一段才写。
#[test]
fn candidates_scattered_among_ineligible_ones_are_kept_across_steps() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 0..2000);
    let cutoff = ArchiveCutoff {
        sealed_offset: 2 * 160,
        ..later()
    };
    let packed = round(&mut broker, cutoff, ArchiveBatch::Usual).unwrap();
    assert_eq!((packed, fixture.segments().len()), (161, 3));
}

/// 平时凑不够半段不写；有一处到八成可以写小段，但只有几包的仍不写；离线命令剩多少写多少。
#[test]
fn how_small_a_segment_may_be_depends_on_how_full_the_store_is() {
    for (batch, count, expected) in [
        (ArchiveBatch::Usual, 31, 0),
        (ArchiveBatch::Usual, 32, 32),
        (ArchiveBatch::Pressed, 7, 0),
        (ArchiveBatch::Pressed, 8, 8),
        (ArchiveBatch::All, 1, 1),
    ] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 1);
        bulk(&broker, 1..count);
        let before = fs::read(fixture.data.join("writer-state.json")).unwrap();
        assert_eq!(
            round(&mut broker, later(), batch).unwrap(),
            expected,
            "{batch:?} {count}"
        );
        if expected == 0 {
            // 凑不够就什么都不动：状态、散件、目录和没打过一样。
            assert_eq!(
                fs::read(fixture.data.join("writer-state.json")).unwrap(),
                before
            );
            assert!(!fixture.data.join("archive-v1").exists());
            assert_eq!(loose_records(&fixture.data).len(), 4 * count);
        }
    }
}

/// 游标可以跨轮留着；凑够之前变得不合格、或者读不回来的，写的那一步重核掉，不写旧的。
#[test]
fn held_candidates_are_checked_again_before_they_are_written() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 1..40);
    let mut cursor = ArchiveCursor::default();
    let step = broker
        .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    assert_eq!((step.packed, step.done), (0, false));
    // 判据退回去（时钟倒退）：一个都不合格了，一段都不写，这一轮就完了。
    let early = ArchiveCutoff {
        now_ms: now() - 30 * DAY,
        ..later()
    };
    let step = broker
        .archive_step(early, ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    assert_eq!((step.packed, step.done), (0, true));
    assert!(!fixture.data.join("archive-v1").exists());
    // 掉了几个、剩下的仍够一段：写剩下的。一条散件在这之间坏了：这一步报损坏，什么都不删，游标从头再来。
    let mut cursor = ArchiveCursor::default();
    broker
        .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    let id = crate::recorded::store::id(&broker, &sha256(b"bulk-3"));
    let ack = fixture
        .data
        .join(format!("history/meta-v1/requests/{id}-outcome.json.ack"));
    let kept = fs::read(&ack).unwrap();
    fs::write(&ack, b"{}").unwrap();
    let before = loose_records(&fixture.data);
    assert!(
        broker
            .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
            .is_err()
    );
    assert_eq!(loose_records(&fixture.data), before);
    assert!(!fixture.data.join("archive-v1").exists());
    fs::remove_file(&ack).unwrap();
    let step = broker
        .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    assert_eq!((step.examined, step.packed), (40, 0));
    // 少一条散件的不算合格，剩 39 条。
    let step = broker
        .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    assert_eq!((step.packed, step.done), (39, true));
    fs::write(&ack, kept).unwrap();
    assert_eq!(pack(&mut broker, later()), 1);
}

/// 挑完到写之间掉了几个：剩下的还够半段就写剩下的；不够就这一轮不写，不因为挑的时候够了就写一个小段。
#[test]
fn a_batch_that_shrinks_is_written_only_if_enough_is_left() {
    for (count, expected) in [(34, 32), (33, 0)] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 1);
        bulk(&broker, 1..count);
        let mut cursor = ArchiveCursor::default();
        broker
            .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
            .unwrap();
        for index in [3, 4] {
            let key = sha256(format!("bulk-{index}").as_bytes());
            let id = crate::recorded::store::id(&broker, &key);
            fs::remove_file(
                fixture
                    .data
                    .join(format!("history/meta-v1/requests/{id}-outcome.json.ack")),
            )
            .unwrap();
        }
        let step = broker
            .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
            .unwrap();
        assert_eq!((step.packed, step.done), (expected, true), "{count}");
        assert_eq!(fixture.segments().len(), usize::from(expected > 0));
    }
}

/// 有在途的事这一步不做，挑到的还留着，等它收口接着来。
#[test]
fn an_unresolved_write_keeps_what_was_already_found() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 1..40);
    let mut cursor = ArchiveCursor::default();
    broker
        .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    broker.mutate_recorded(note(5), metadata()).unwrap();
    assert!(matches!(
        broker.archive_step(later(), ArchiveBatch::Usual, &mut cursor),
        Err(BrokerError::MutationUnresolved)
    ));
    flush(&mut broker);
    let step = broker
        .archive_step(later(), ArchiveBatch::Usual, &mut cursor)
        .unwrap();
    assert_eq!((step.examined, step.packed), (0, 40));
}

/// 记录大的按字节装满：不到 64 包也是满段，装不下的那个留到下一段，一个不漏。
#[test]
fn big_records_fill_a_segment_by_bytes() {
    let biggest = 4 * crate::writer::disk::RECORD_LIMIT;
    let mut cursor = ArchiveCursor::default();
    let held = (0..64)
        .take_while(|_| cursor.hold(format::Kind::Content, "a", biggest))
        .count();
    assert_eq!(held, 31);
    assert!(cursor.full() && cursor.ready(ArchiveBatch::Usual));
    assert_eq!(cursor.take().len(), 31);
    assert!(!cursor.ready(ArchiveBatch::All));
    assert!(cursor.hold(format::Kind::Content, "b", biggest));
    assert!(!cursor.full());
    // 真写：两份回执和重放各垫到 64000 字节（描述符只认唯一写法，垫不了），一包 19 万多，一段装得下 42 包。
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 1..50);
    for path in loose::DIRECTORIES {
        let Ok(entries) = fs::read_dir(fixture.data.join(path)) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.parent().unwrap().ends_with("requests") && path.extension().unwrap() != "ack" {
                continue;
            }
            let mut bytes = fs::read(&path).unwrap();
            bytes.resize(64_000, b' ');
            fs::write(&path, bytes).unwrap();
        }
    }
    let before = all_records(&broker);
    let mut written = Vec::new();
    let mut cursor = ArchiveCursor::default();
    loop {
        let step = broker
            .archive_step(later(), ArchiveBatch::All, &mut cursor)
            .unwrap();
        written.extend((step.packed > 0).then_some(step.packed));
        if step.done {
            break;
        }
    }
    assert_eq!(written, [42, 8]);
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(all_records(&broker), before);
}

/// 用量到一半按平时的批量，到八成放小段，到 95% 一包也写，不到一半不打。
#[test]
fn the_batch_follows_the_fullest_place() {
    let at = |used| RecordUsage {
        descriptors: RecordUsageItem {
            used: 1,
            limit: 100,
        },
        descriptor_bytes: RecordUsageItem {
            used: 1,
            limit: 100,
        },
        replays: RecordUsageItem { used, limit: 100 },
        replay_bytes: RecordUsageItem {
            used: 1,
            limit: 100,
        },
        file_operations: RecordUsageItem {
            used: 1,
            limit: 100,
        },
        file_operation_bytes: RecordUsageItem {
            used: 1,
            limit: 100,
        },
        tombstones: RecordUsageItem {
            used: 100,
            limit: 100,
        },
        ..RecordUsage::default()
    };
    for (used, expected) in [
        (49, None),
        (50, Some(ArchiveBatch::Usual)),
        (79, Some(ArchiveBatch::Usual)),
        (80, Some(ArchiveBatch::Pressed)),
        (94, Some(ArchiveBatch::Pressed)),
        (95, Some(ArchiveBatch::All)),
        (100, Some(ArchiveBatch::All)),
    ] {
        assert_eq!(at(used).archive_batch(), expected, "{used}");
    }
}

/// 没有上限的一处（没启用）永远没到；数再大也不溢出。
#[test]
fn a_place_without_a_limit_is_never_full_and_huge_numbers_do_not_overflow() {
    let item = |used, limit| RecordUsageItem { used, limit };
    assert!(!item(0, 0).nearly_full());
    assert!(!item(5, 0).nearly_full());
    assert!(item(u64::MAX, u64::MAX).nearly_full());
    assert!(!item(u64::MAX / 2, u64::MAX).nearly_full());
    // 4096 的八成是 3276.8：3277 才算到。
    assert!(!item(3_276, 4_096).nearly_full());
    assert!(item(3_277, 4_096).nearly_full());
    assert_eq!(RecordUsage::default().archive_batch(), None);
    let huge = RecordUsage {
        replay_bytes: item(u64::MAX, u64::MAX),
        ..RecordUsage::default()
    };
    assert_eq!(huge.archive_batch(), Some(ArchiveBatch::All));
}
