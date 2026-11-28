use super::*;

#[test]
fn packed_records_answer_exactly_as_before_and_stay_byte_for_byte() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 5);
    let first = broker.read(&logical("note-0.md")).unwrap().revision;
    broker
        .mutate_recorded(
            Mutation::replace(logical("note-0.md"), b"new".to_vec(), first, "replace"),
            metadata(),
        )
        .unwrap();
    flush(&mut broker);
    let before = all_records(&broker);
    let history = broker
        .list_history(&logical("note-0.md"), 10, None)
        .unwrap();
    assert_eq!(pack(&mut broker, later()), 6);
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(all_records(&broker), before);
    assert_eq!(fixture.state()["format_version"], 9);
    for reopened in [false, true] {
        if reopened {
            drop(broker);
            broker = fixture.reopen();
        }
        for index in 0..5 {
            answers(&broker, index);
        }
        // 历史证据要装回描述符、核重放：打包前后一样。
        assert_eq!(
            broker
                .list_history(&logical("note-0.md"), 10, None)
                .unwrap(),
            history
        );
        assert_eq!(
            broker
                .read_history(&logical("note-0.md"), first, 1024)
                .unwrap()
                .content,
            "正文 0".as_bytes()
        );
    }
    // 旧键再来一次是重放，不会再造散件，也不会再装一遍。
    broker.mutate_recorded(note(1), metadata()).unwrap();
    assert!(loose_records(&fixture.data).is_empty());
    // 打包之后照常写，新记录是散件；老篇目接着改，历史接得上。
    broker.mutate_recorded(note(9), metadata()).unwrap();
    let second = broker.read(&logical("note-0.md")).unwrap().revision;
    broker
        .mutate_recorded(
            Mutation::replace(logical("note-0.md"), b"third".to_vec(), second, "third"),
            metadata(),
        )
        .unwrap();
    flush(&mut broker);
    assert_eq!(loose_records(&fixture.data).len(), 8);
    assert!(
        broker
            .list_history(&logical("note-0.md"), 10, None)
            .unwrap()
            .entries
            .len()
            > history.entries.len()
    );
    // 第二轮接在链上。
    assert_eq!(pack(&mut broker, later()), 2);
    assert_eq!(fixture.state()["archive"]["segments"], 2);
    assert_eq!(fixture.state()["archive"]["packs"], 8);
}

#[test]
fn a_store_that_was_never_packed_keeps_its_state_bytes() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    let before = fs::read(fixture.data.join("writer-state.json")).unwrap();
    // 都没满 7 天：一包也不打，状态一个字节不变，也不建目录。
    let cutoff = ArchiveCutoff {
        now_ms: now() + DAY,
        ..later()
    };
    assert_eq!(pack(&mut broker, cutoff), 0);
    drop(broker);
    drop(fixture.reopen());
    assert_eq!(
        fs::read(fixture.data.join("writer-state.json")).unwrap(),
        before
    );
    assert!(!fixture.data.join("archive-v1").exists());
}

/// 没有审计主体的写入没有回执：成功封存还没有内容截止时刻时留散件，照样答得出；有了、请求早于它才打，
/// 打了照样答得出（T-249 G2；T-235 时永远不打）。
#[test]
fn a_write_without_an_actor_waits_for_a_seal_content_cutoff() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    broker.mutate(note(9)).unwrap();
    notes(&mut broker, 2);
    let kept = loose_records(&fixture.data).len();
    let audited_only = ArchiveCutoff {
        sealed_content_ms: None,
        ..later()
    };
    assert_eq!(pack(&mut broker, audited_only), 2);
    assert_eq!(loose_records(&fixture.data).len(), kept - 8);
    assert!(broker.mutation_replay(&note(9)).unwrap().is_some());
    assert_eq!(pack(&mut broker, later()), 1);
    assert_eq!(loose_records(&fixture.data).len(), kept - 10);
    assert!(broker.mutation_replay(&note(9)).unwrap().is_some());
}

#[test]
fn a_path_with_packed_content_but_no_history_catalog_is_still_corrupt() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let catalog = sha256(b"note-0.md");
    fs::remove_file(
        fixture
            .data
            .join(format!("history/meta-v1/catalogs/{catalog}.json")),
    )
    .unwrap();
    assert!(matches!(
        broker.list_history(&logical("note-0.md"), 10, None),
        Err(BrokerError::CorruptWriterState)
    ));
    broker
        .list_history(&logical("note-1.md"), 10, None)
        .unwrap();
}

#[test]
fn a_restore_of_history_still_replays_after_packing() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let path = logical("note-0.md");
    let first = broker.read(&path).unwrap().revision;
    let second = broker
        .mutate_recorded(
            Mutation::replace(path.clone(), b"two".to_vec(), first, "two"),
            metadata(),
        )
        .unwrap()
        .revision;
    flush(&mut broker);
    let actor = MutationActor::Human {
        issuer: "https://issuer.test".into(),
        subject: "human".into(),
    };
    let restore = broker
        .prepare_recorded_restore(
            Mutation::replace(path.clone(), "正文 0".into(), second, "restore"),
            MutationMetadata::new(
                actor.clone(),
                MutationKind::Restore {
                    source_revision: first,
                },
            )
            .unwrap(),
        )
        .unwrap();
    let candidate = broker.pending_content_audit(1).unwrap().remove(0);
    broker.confirm_content_audit(candidate, 50, HASH).unwrap();
    broker.commit_recorded_restore(restore).unwrap();
    flush(&mut broker);
    assert_eq!(pack(&mut broker, later()), 3);
    assert_eq!(
        broker
            .restore_replay(&path, second, first, "restore", &actor)
            .unwrap()
            .unwrap()
            .revision,
        first
    );
    assert!(matches!(
        broker.restore_replay(&path, first, first, "restore", &actor),
        Err(BrokerError::IdempotencyConflict)
    ));
}

/// 拒绝的终态也是一整包：旧键照样答原来的拒绝。
#[test]
fn a_rejected_write_is_packed_and_still_answers_its_rejection() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let mut again = note(0);
    again.idempotency_key = "conflict".into();
    assert!(matches!(
        broker.mutate_recorded(again.clone(), metadata()),
        Err(BrokerError::RevisionConflict { .. })
    ));
    flush(&mut broker);
    assert_eq!(pack(&mut broker, later()), 2);
    assert!(matches!(
        broker.mutate_recorded(again, metadata()),
        Err(BrokerError::RevisionConflict { .. })
    ));
}

/// 经维护收口的请求照样是一整包：维护记录引用的描述符、重放在段里也查得到，封存逐条核得过，旧键答取消。
#[test]
fn a_request_settled_by_maintenance_is_packed_and_its_maintenance_record_still_checks_out() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 1);
    let path = logical("note-0.md");
    let first = broker.read(&path).unwrap().revision;
    let second = broker
        .mutate_recorded(
            Mutation::replace(path.clone(), b"two".to_vec(), first, "two"),
            metadata(),
        )
        .unwrap()
        .revision;
    flush(&mut broker);
    let actor = MutationActor::Human {
        issuer: "https://issuer.test".into(),
        subject: "human".into(),
    };
    drop(
        broker
            .prepare_recorded_restore(
                Mutation::replace(path.clone(), "正文 0".into(), second, "lost"),
                MutationMetadata::new(
                    actor.clone(),
                    MutationKind::Restore {
                        source_revision: first,
                    },
                )
                .unwrap(),
            )
            .unwrap(),
    );
    let review = broker.maintenance_inspect(64).unwrap().targets[0]
        .review_id
        .clone();
    let ticket = broker
        .prepare_maintenance(
            &review,
            MaintenanceDecision::CancelRestoreIntent,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    broker
        .confirm_maintenance_intent(&ticket, 90, HASH)
        .unwrap();
    broker.commit_maintenance(ticket).unwrap();
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(candidate, 91, HASH)
            .unwrap();
    }
    flush(&mut broker);
    let replay = broker.restore_replay(&path, second, first, "lost", &actor);
    assert!(replay.is_err() || replay.as_ref().unwrap().is_none());
    let said = format!("{replay:?}");
    assert_eq!(pack(&mut broker, later()), 3);
    assert!(loose_records(&fixture.data).is_empty());
    assert_eq!(
        format!(
            "{:?}",
            broker.restore_replay(&path, second, first, "lost", &actor)
        ),
        said
    );
    let (_, sealed) = checkpoint(&fixture, broker, "seal");
    // 三条请求各两份回执，外加维护的两份。
    assert_eq!(sealed.unwrap(), 8);
}
