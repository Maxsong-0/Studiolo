use super::*;

#[test]
fn a_seal_checks_every_packed_record_on_the_copy() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 4);
    pack(&mut broker, later());
    broker.mutate_recorded(note(8), metadata()).unwrap();
    flush(&mut broker);
    let (broker, sealed) = checkpoint(&fixture, broker, "seal");
    assert_eq!(sealed.unwrap(), 10);
    answers(&broker, 0);
    drop(broker);
    // 副本里的段是正常的 canonical 写者状态，恢复到新目录不用重绑定，旧键照样答。
    let (left, right) = (
        fixture.temp.path().join("seal-vault"),
        fixture.temp.path().join("seal-data"),
    );
    let state = fixture.state();
    let copy = FsVaultBroker::open_existing(&left, &right);
    assert!(copy.is_err(), "没重绑定就打开了");
    let binding: serde_json::Value =
        serde_json::from_slice(&fs::read(right.join("broker-binding.json")).unwrap()).unwrap();
    rebind_new_roots(&NewRootRebind {
        new_vault_root: &left,
        new_data_root: &right,
        expected_vault_uuid: serde_json::from_value(binding["vault_uuid"].clone()).unwrap(),
        expected_pair_id: serde_json::from_value(binding["pair_id"].clone()).unwrap(),
        checkpoint_id: "checkpoint-archive".into(),
        restore_id: "restore-archive",
        parent_lineage: None,
        resume_interrupted: false,
    })
    .unwrap();
    let mut copy = FsVaultBroker::open_existing(&left, &right).unwrap();
    assert!(state["archive"].is_object());
    for index in [0, 1, 2, 3, 8] {
        answers(&copy, index);
    }
    // 恢复出来的接着打、接着在原链上。
    assert_eq!(pack(&mut copy, later()), 1);
}

fn flip(path: &Path, at: usize) {
    let mut bytes = fs::read(path).unwrap();
    bytes[at] ^= 1;
    fs::write(path, bytes).unwrap();
}

/// 定格之前段被动过手脚：改一个字节、少一段、多一段、断了链、散件和段不一致，都封不成。
#[test]
fn a_tampered_archive_never_produces_a_ticket() {
    type Plant = fn(&Fixture);
    let samples: [(&str, Plant); 8] = [
        ("头里改一个字节", |fixture| {
            flip(&fixture.segments()[0], 40);
        }),
        ("内容里改一个字节", |fixture| {
            let path = fixture.segments()[1].clone();
            let last = fs::metadata(&path).unwrap().len() as usize - 2;
            flip(&path, last);
        }),
        ("少最后一段", |fixture| {
            fs::remove_file(&fixture.segments()[1]).unwrap();
        }),
        ("少第一段", |fixture| {
            fs::remove_file(&fixture.segments()[0]).unwrap();
        }),
        ("多一段", |fixture| {
            let path = fixture.segments()[1].clone();
            let name = format!("00000003-{}.seg", "0".repeat(64));
            fs::copy(&path, path.with_file_name(name)).unwrap();
        }),
        ("多一个认不得的文件", |fixture| {
            fs::write(fixture.data.join("archive-v1/extra"), b"x").unwrap();
        }),
        ("段尾多一个字节", |fixture| {
            let path = fixture.segments()[0].clone();
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b' ');
            fs::write(path, bytes).unwrap();
        }),
        ("散件又冒出来、内容不一样", |fixture| {
            let key = sha256(note(0).idempotency_key.as_bytes());
            fs::write(
                fixture.data.join(format!("idempotency/{key}.json")),
                b"{\"format_version\":4}",
            )
            .unwrap();
        }),
    ];
    for (name, plant) in samples {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 2);
        pack(&mut broker, later());
        broker.mutate_recorded(note(8), metadata()).unwrap();
        flush(&mut broker);
        pack(&mut broker, later());
        assert_eq!(fixture.segments().len(), 2);
        plant(&fixture);
        let (broker, sealed) = checkpoint(&fixture, broker, "seal");
        assert!(sealed.is_err(), "{name}");
        drop(broker);
        // 重开也不认；只有「多一段」里那段能核同才清，这里核不同，一样拒。
        assert!(fixture.try_reopen().is_err(), "重开：{name}");
    }
}

/// 状态里的段头被换回旧的（链短了一截）：后一段变成孤儿，里面的散件不在，不许当没发生。
#[test]
fn rolling_the_head_back_is_refused() {
    for before_second_write in [true, false] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 2);
        pack(&mut broker, later());
        let state = fixture.data.join("writer-state.json");
        let old = fs::read(&state).unwrap();
        broker.mutate_recorded(note(8), metadata()).unwrap();
        flush(&mut broker);
        let middle = fs::read(&state).unwrap();
        pack(&mut broker, later());
        fs::write(&state, if before_second_write { old } else { middle }).unwrap();
        assert!(broker.write_status().is_err());
        drop(broker);
        assert!(fixture.try_reopen().is_err());
    }
}

/// 账本里没有原事件、或者和原回执不是同一条：封不成。
#[test]
fn a_packed_receipt_the_ledger_does_not_have_is_refused() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let mut frozen = broker.freeze_checkpoint(deadline());
    frozen.capture().unwrap();
    let exported = frozen
        .take_capture()
        .unwrap()
        .export(
            &fixture.target("left"),
            &fixture.target("right"),
            deadline(),
        )
        .unwrap();
    let mut seen = 0;
    assert!(
        exported
            .verify(deadline(), |_, _, _| {
                seen += 1;
                if seen == 4 {
                    Err(BrokerError::CorruptWriterState)
                } else {
                    Ok(())
                }
            })
            .is_err()
    );
    assert_eq!(seen, 4);
}

/// 恢复目录里的段被动过：不恢复。
#[test]
fn a_restore_refuses_a_tampered_archive() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let (broker, sealed) = checkpoint(&fixture, broker, "seal");
    sealed.unwrap();
    let (vault, pair) = (broker.vault_uuid, broker.writer.binding.pair_id);
    let right = fixture.temp.path().join("seal-data");
    let segment = fs::read_dir(right.join("archive-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    flip(&segment, 60);
    assert!(
        rebind_new_roots(&NewRootRebind {
            new_vault_root: &fixture.temp.path().join("seal-vault"),
            new_data_root: &right,
            expected_vault_uuid: vault,
            expected_pair_id: pair,
            checkpoint_id: "checkpoint-archive".into(),
            restore_id: "restore-archive",
            parent_lineage: None,
            resume_interrupted: false,
        })
        .is_err()
    );
}

/// 受信导出给段文件归类：和其余写者状态一样，不用改分类。
#[test]
fn the_trusted_export_classifies_segments_as_writer_state() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let (broker, sealed) = checkpoint(&fixture, broker, "seal");
    sealed.unwrap();
    let export = fixture.target("export");
    fs::rename(
        fixture.temp.path().join("seal-vault"),
        export.join("canonical"),
    )
    .unwrap();
    fs::rename(
        fixture.temp.path().join("seal-data"),
        export.join("writer-data"),
    )
    .unwrap();
    let facts = read_trusted_export(&export, broker.vault_uuid).unwrap();
    let segments: Vec<_> = facts
        .iter()
        .filter(|fact| fact.export_path.as_str().contains("archive-v1"))
        .collect();
    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].class,
        studiolo_core::TrustedExportClass::Canonical
    ));
}
