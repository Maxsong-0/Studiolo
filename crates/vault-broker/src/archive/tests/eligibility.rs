use super::*;

/// 请求时间满 7 天才打；差一毫秒不打。时钟倒退不打。
#[test]
fn only_requests_that_are_seven_days_old_are_packed() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let requests = loose_records(&fixture.data);
    let at = requests
        .iter()
        .find(|((path, name), _)| path.ends_with("requests") && !name.ends_with(".ack"))
        .map(|(_, bytes)| serde_json::from_slice::<serde_json::Value>(bytes).unwrap())
        .unwrap()["requested_at_ms"]
        .as_i64()
        .unwrap();
    for (now_ms, expected) in [
        (at - 8 * DAY, 0),
        (i64::MIN, 0),
        (at + 7 * DAY - 1, 0),
        (at + 7 * DAY, 1),
    ] {
        let cutoff = ArchiveCutoff { now_ms, ..later() };
        assert_eq!(pack(&mut broker, cutoff), expected, "{now_ms}");
    }
}

/// 两份回执都在封存水位里才打：意图 1、结果 2，水位 1 不够。
#[test]
fn both_receipts_must_be_within_the_sealed_watermark() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    for (sealed_offset, expected) in [(0, 0), (1, 0), (2, 1)] {
        let cutoff = ArchiveCutoff {
            sealed_offset,
            ..later()
        };
        assert_eq!(pack(&mut broker, cutoff), expected, "{sealed_offset}");
    }
}

/// 有在途的事一律不打，也不动任何东西。
#[test]
fn nothing_is_packed_while_anything_is_unresolved() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    // 待投递没清空。
    broker.mutate_recorded(note(5), metadata()).unwrap();
    let before = loose_records(&fixture.data);
    assert!(matches!(
        round(&mut broker, later(), ArchiveBatch::All),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(loose_records(&fixture.data), before);
    flush(&mut broker);
    // 写到一半。
    fault(&broker, 1);
    assert!(broker.mutate_recorded(note(6), metadata()).is_err());
    assert!(matches!(
        round(&mut broker, later(), ArchiveBatch::All),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(!fixture.data.join("archive-v1").exists());
    // 没有审计主体的写到一半：没有待投递，也一样不打。
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    fault(&broker, 1);
    assert!(broker.mutate(note(6)).is_err());
    assert!(broker.pending_content_audit(64).unwrap().is_empty());
    assert!(matches!(
        round(&mut broker, later(), ArchiveBatch::All),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(!fixture.data.join("archive-v1").exists());
}

/// 坏记录不打包，也不跳过：当场报损坏，什么都不删。
#[test]
fn a_damaged_record_stops_the_step_and_removes_nothing() {
    for (suffix, field) in [
        ("-intent.json.ack", "digest"),
        ("-outcome.json.ack", "event_id"),
    ] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 3);
        let path = fs::read_dir(fixture.data.join("history/meta-v1/requests"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_str().unwrap().ends_with(suffix))
            .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value[field] = "b".repeat(64).into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let before = loose_records(&fixture.data);
        assert!(round(&mut broker, later(), ArchiveBatch::All).is_err());
        assert_eq!(loose_records(&fixture.data), before);
    }
}

/// 终态说的不是这条描述符的请求（指着同一条母本、路径却换了）：报损坏，不打。
#[test]
fn a_terminal_that_tells_another_story_is_not_packed() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    let key = sha256(note(0).idempotency_key.as_bytes());
    let path = fixture.data.join(format!("idempotency/{key}.json"));
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("note-0.md").count(), 1);
    fs::write(&path, text.replace("note-0.md", "note-9.md")).unwrap();
    let before = loose_records(&fixture.data);
    assert!(matches!(
        round(&mut broker, later(), ArchiveBatch::All),
        Err(BrokerError::CorruptWriterState)
    ));
    assert_eq!(loose_records(&fixture.data), before);
}

/// 缺一份回执、缺重放的不是一整包：留着不打，别的照打。
#[test]
fn an_incomplete_set_is_left_alone() {
    for missing in [0, 1] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 3);
        let request = note(1);
        let key = sha256(request.idempotency_key.as_bytes());
        let id = crate::recorded::store::id(&broker, &key);
        fs::remove_file(match missing {
            0 => fixture
                .data
                .join(format!("history/meta-v1/requests/{id}-outcome.json.ack")),
            _ => fixture.data.join(format!("idempotency/{key}.json")),
        })
        .unwrap();
        assert_eq!(pack(&mut broker, later()), 2);
        assert_eq!(loose_records(&fixture.data).len(), 3);
    }
}

/// 一步只做一样：最多看 256 个，或者写最多 64 包；游标接着走，不重不漏。
#[test]
fn a_round_is_cut_into_bounded_steps() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    bulk(&broker, 0..150);
    let mut steps = Vec::new();
    let mut cursor = ArchiveCursor::default();
    loop {
        let step = broker
            .archive_step(later(), ArchiveBatch::All, &mut cursor)
            .unwrap();
        steps.push((step.examined, step.packed));
        if step.done {
            break;
        }
    }
    assert_eq!(
        steps,
        [(64, 0), (0, 64), (64, 0), (0, 64), (23, 0), (0, 23)]
    );
    // 没有一个合格的时候按看过的数分步。
    bulk(&broker, 150..750);
    let none = ArchiveCutoff {
        sealed_offset: 0,
        ..later()
    };
    let mut cursor = ArchiveCursor::default();
    let mut examined = Vec::new();
    loop {
        let step = broker
            .archive_step(none, ArchiveBatch::All, &mut cursor)
            .unwrap();
        assert_eq!(step.packed, 0);
        examined.push(step.examined);
        if step.done {
            break;
        }
    }
    assert_eq!(examined, [256, 256, 88]);
}
