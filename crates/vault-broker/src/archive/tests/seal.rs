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

/// 把第一包里的这一条放回散件，尾上多一个空格：解得开、说的也一样，只是字节不同。
pub(super) fn put_back(fixture: &Fixture, part: Part) {
    let state: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.data.join("broker-binding.json")).unwrap())
            .unwrap();
    let id = |name: &str| serde_json::from_value(state[name].clone()).unwrap();
    let bytes = fs::read(&fixture.segments()[0]).unwrap();
    let segment = format::decode(&bytes, id("pair_id"), id("vault_uuid")).unwrap();
    for (pack, _) in segment.packs {
        let Some(at) = pack.kind.parts().iter().position(|own| *own == part) else {
            continue;
        };
        let mut bytes = pack.bytes[at].clone();
        bytes.push(b' ');
        let name = part.file_name(pack.name_of(part));
        fs::write(fixture.data.join(part.directory()).join(name), bytes).unwrap();
        return;
    }
    panic!("这一段里没有这种记录");
}

/// 定格之前段被动过手脚：改一个字节、少一段、多一段、断了链、散件和段不一致，都封不成。
#[test]
fn a_tampered_archive_never_produces_a_ticket() {
    type Plant = fn(&Fixture);
    let samples: [(&str, Plant); 11] = [
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
        ("描述符散件又冒出来、内容不一样", |fixture| {
            let id = fs::read(fixture.segments()[0].clone()).unwrap();
            let header = String::from_utf8_lossy(&id).into_owned();
            let at = header.find("\"id\":\"").unwrap() + 6;
            fs::write(
                fixture.data.join(format!(
                    "history/meta-v1/requests/{}.json",
                    &header[at..at + 64]
                )),
                b"{}",
            )
            .unwrap();
        }),
        ("回执散件又冒出来、只多一个空格", |fixture| {
            put_back(fixture, Part::IntentAck);
        }),
        ("重放散件又冒出来、只多一个空格", |fixture| {
            put_back(fixture, Part::Replay);
        }),
        ("重放散件又冒出来、内容不一样", |fixture| {
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

/// 在线读一条也核它的摘要：开库之后段被改了一个字节，那一条报损坏，别的照答。
#[test]
fn a_live_read_checks_the_bytes_it_takes_from_a_segment() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let segment = fixture.segments()[0].clone();
    // 换掉最后一条重放里序号的一位：还是合法的一条，和描述符也对得上，只有摘要看得出来。
    let mut bytes = fs::read(&segment).unwrap();
    let mark = br#""sequence":"#;
    let at = bytes
        .windows(mark.len())
        .rposition(|window| window == mark)
        .unwrap()
        + mark.len();
    bytes[at] = if bytes[at] == b'5' { b'6' } else { b'5' };
    fs::write(&segment, bytes).unwrap();
    let archive = broker.archive().unwrap();
    // 最后一条是最后一包的重放。
    let key = archive.packs().last().unwrap().key.clone();
    let mut damaged = 0;
    for index in 0..2 {
        let own = sha256(note(index).idempotency_key.as_bytes()) == key;
        let answer = broker.mutation_replay_recorded(&note(index), &metadata());
        if own {
            assert!(matches!(answer, Err(BrokerError::CorruptWriterState)));
            damaged += 1;
        } else {
            answers(&broker, index);
        }
    }
    assert_eq!(damaged, 1);
}

/// 状态里的段头每一项都要和真的链对上：包数、段数、链尾改一项都不开；第 9 版必须有段头，有段头必须是第 9 版。
#[test]
fn every_field_of_the_head_must_match_the_chain() {
    type Change = fn(&mut serde_json::Value);
    let changes: [(&str, Change); 6] = [
        ("包数多一", |state| state["archive"]["packs"] = 4.into()),
        ("包数少一", |state| state["archive"]["packs"] = 2.into()),
        ("段数多一", |state| {
            state["archive"]["segments"] = 2.into()
        }),
        ("链尾换了", |state| {
            state["archive"]["tail"] = "0".repeat(64).into();
        }),
        ("第 9 版没有段头", |state| {
            state.as_object_mut().unwrap().remove("archive");
        }),
        ("有段头却是第 8 版", |state| {
            state["format_version"] = 8.into()
        }),
    ];
    for (name, change) in changes {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        notes(&mut broker, 3);
        pack(&mut broker, later());
        drop(broker);
        let mut state = fixture.state();
        change(&mut state);
        fs::write(
            fixture.data.join("writer-state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        assert!(fixture.try_reopen().is_err(), "{name}");
    }
}

/// 同一条记录打进两段：摘要链自己说得通也不认。
#[test]
fn a_record_packed_twice_is_refused() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let (pair, vault) = (broker.writer.binding.pair_id, broker.vault_uuid);
    let segment = fixture.segments()[0].clone();
    let bytes = fs::read(&segment).unwrap();
    let tail = sha256(&bytes);
    let packs: Vec<_> = format::decode(&bytes, pair, vault)
        .unwrap()
        .packs
        .into_iter()
        .map(|(pack, _)| pack)
        .collect();
    let again = format::encode(
        &format::Place {
            pair,
            vault,
            sequence: 2,
            previous: Some(&tail),
        },
        &packs,
    )
    .unwrap();
    let name = format::file_name(2, &sha256(&again));
    fs::write(segment.with_file_name(&name), &again).unwrap();
    fs::set_permissions(
        segment.with_file_name(&name),
        fs::metadata(&segment).unwrap().permissions(),
    )
    .unwrap();
    drop(broker);
    let mut state = fixture.state();
    state["archive"] = serde_json::json!({"segments":2,"packs":4,"tail":sha256(&again)});
    fs::write(
        fixture.data.join("writer-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        fixture.try_reopen(),
        Err(BrokerError::CorruptWriterState)
    ));
}

/// 段的编码只有一种写法：头里多一个空格、尾上多一个字节、文件名和内容对不上，都不认。
#[test]
fn a_segment_has_exactly_one_spelling_and_its_name_is_its_digest() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let (pair, vault) = (broker.writer.binding.pair_id, broker.vault_uuid);
    let segment = fixture.segments()[0].clone();
    let bytes = fs::read(&segment).unwrap();
    let mut longer = bytes.clone();
    longer.push(b' ');
    assert!(format::decode(&longer, pair, vault).is_err());
    let length = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) + 1;
    let mut spaced = bytes[..8].to_vec();
    spaced.extend_from_slice(&length.to_be_bytes());
    spaced.extend_from_slice(b"{ ");
    spaced.extend_from_slice(&bytes[13..]);
    assert!(format::decode(&spaced, pair, vault).is_err());
    drop(broker);
    fs::rename(
        &segment,
        segment.with_file_name(format::file_name(1, &"0".repeat(64))),
    )
    .unwrap();
    let mut state = fixture.state();
    state["archive"]["tail"] = "0".repeat(64).into();
    fs::write(
        fixture.data.join("writer-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert!(fixture.try_reopen().is_err());
}

/// 段头里换一个人、换一个库、换一条记录的位置，都不是这一对写者的段。
#[test]
fn a_segment_of_another_store_is_refused() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    notes(&mut broker, 2);
    pack(&mut broker, later());
    let (pair, vault) = (broker.writer.binding.pair_id, broker.vault_uuid);
    let bytes = fs::read(&fixture.segments()[0]).unwrap();
    assert!(format::decode(&bytes, pair, vault).is_ok());
    assert!(format::decode(&bytes, vault, vault).is_err());
    assert!(format::decode(&bytes, pair, pair).is_err());
    let mut swapped = format::decode(&bytes, pair, vault).unwrap().packs;
    let (first, _) = swapped.remove(0);
    let (mut second, _) = swapped.remove(0);
    // 第二包的重放换成第一包的：逐条摘要都对，但说的不是同一个请求。
    second.bytes[3].clone_from(&first.bytes[3]);
    let forged = format::encode(
        &format::Place {
            pair,
            vault,
            sequence: 1,
            previous: None,
        },
        &[second],
    )
    .unwrap();
    let segment = format::decode(&forged, pair, vault).unwrap();
    let mut archive = super::super::Archive::default();
    assert!(
        archive
            .append(
                format::file_name(1, &sha256(&forged)),
                forged.len() as u64,
                &segment,
                pair
            )
            .is_err()
    );
}
