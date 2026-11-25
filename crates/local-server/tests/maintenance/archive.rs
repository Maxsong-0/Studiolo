//! 写者旧记录的离线打包与还原（BUG-051）：水位只认成功的封存，没确认不动，有在途的事不动，还原回原样。
use super::{
    super::{
        MaintenanceError, archive, flush, inputs::Inputs, options::Command, workflow::execute,
    },
    fixture::Fixture,
};
use serde_json::{Value, json};
use std::{num::NonZeroU64, path::Path};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, IntegrityHash, SanitizedPayload,
    SqliteAuditLedger,
};
use studiolo_backup::worker::{
    job::{BACKUP_AUDIT_ACTION, backup_intent},
    state::JobKey,
};
use studiolo_core::{VaultPath, checkpoint::CheckpointId};
use studiolo_vault_broker::{
    MaintenanceActor, Mutation, MutationActor, MutationKind, MutationMetadata,
};

const DAY: i64 = 24 * 60 * 60 * 1000;

fn accept(_: &Value, _: &str) -> Result<(), MaintenanceError> {
    Ok(())
}
fn check() -> Result<(), MaintenanceError> {
    Ok(())
}
fn actor() -> MaintenanceActor {
    MaintenanceActor::current().unwrap()
}
fn later() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    i64::try_from(now.as_millis()).unwrap() + 8 * DAY
}
fn note(index: usize) -> Mutation {
    Mutation::create(
        VaultPath::parse(format!("new-{index}.md")).unwrap(),
        format!("正文 {index}").into_bytes(),
        format!("key-{index}"),
    )
}
fn metadata() -> MutationMetadata {
    MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap()
}
/// 真写三篇、回执真进账本。
fn written(fixture: &Fixture) -> Inputs {
    let mut inputs = Inputs::open(&fixture.options(Command::ArchivePack)).unwrap();
    for index in 0..3 {
        inputs
            .broker
            .mutate_recorded(note(index), metadata())
            .unwrap();
    }
    flush::all(&mut inputs, &check).unwrap();
    inputs
}
/// 一次封存做成了：原意图、准入（盖到原意图为止）、验过的成功结果。
fn sealed(audit: &Path) -> u64 {
    let ledger = SqliteAuditLedger::open_existing(audit).unwrap();
    let key = JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("scheduled-1").unwrap(),
    };
    let target = crate::backup::producer::TARGET;
    let cutoff = ledger.append(backup_intent(&key, target).unwrap()).unwrap();
    let hash = IntegrityHash::sha256(key.checkpoint_id.as_str().as_bytes()).to_hex();
    let mut events = vec![(
        format!("backup-admission-{hash}"),
        AuditPhase::Intent,
        json!({
            "schema":1,"key":key,"target":target,
            "source":{"checkpoint_id":key.checkpoint_id,"seal_sha256":"ab".repeat(32)},
            "content_cutoff":0,"expires_at":i64::MAX,
            "audit_offset":cutoff.offset(),"audit_hash":cutoff.record_hash().to_hex(),
        }),
    )];
    events.push((
        "backup.snapshot:1:scheduled-1".to_owned(),
        AuditPhase::Outcome,
        json!({"result":"succeeded","objects_complete":true,"checksum_verified":true,
            "paired_checkpoint_reference":true,"job_sequence":1,"checkpoint_id":"scheduled-1"}),
    ));
    for (id, phase, body) in events {
        ledger
            .append(AuditEvent::new(
                AuditEventId::parse(id).unwrap(),
                phase,
                BACKUP_AUDIT_ACTION,
                SanitizedPayload::try_from(body).unwrap(),
            ))
            .unwrap();
    }
    cutoff.offset()
}
fn loose(fixture: &Fixture) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    for path in ["history/meta-v1/requests", "idempotency"] {
        for entry in std::fs::read_dir(fixture.root.join("data").join(path)).unwrap() {
            let entry = entry.unwrap();
            found.push((
                format!("{path}/{}", entry.file_name().to_str().unwrap()),
                std::fs::read(entry.path()).unwrap(),
            ));
        }
    }
    found.sort();
    found
}

#[test]
fn packing_then_unpacking_offline_keeps_every_record_and_every_answer() {
    let fixture = Fixture::new(true, true);
    let mut inputs = written(&fixture);
    let before = loose(&fixture);
    assert_eq!(before.len(), 12);
    // 没有一次成功的封存：没有水位，不打，也不进确认。
    assert!(matches!(
        archive::pack_at(
            &mut inputs,
            &actor(),
            later(),
            &mut |_, _| panic!("没有水位不进确认"),
            &check
        ),
        Err(MaintenanceError::ArchiveNoSeal)
    ));
    let offset = sealed(&fixture.root.join("audit"));
    // 没满 7 天：真实时钟下一条也不打。
    let options = fixture.options(Command::ArchivePack);
    let none = execute(&mut inputs, &options, actor(), accept, check).unwrap();
    assert_eq!(none["packed"], 0);
    assert_eq!(loose(&fixture), before);
    // 没确认就什么都不动。
    assert!(matches!(
        archive::pack_at(
            &mut inputs,
            &actor(),
            later(),
            &mut |_, _| Err(MaintenanceError::Confirmation),
            &check
        ),
        Err(MaintenanceError::Confirmation)
    ));
    assert_eq!(loose(&fixture), before);
    let packed = archive::pack_at(
        &mut inputs,
        &actor(),
        later(),
        &mut |review, expected| {
            assert_eq!(expected, "archive-pack");
            assert_eq!(review["sealed_offset"], offset);
            assert_eq!(review["usage"]["descriptors"]["used"], 3);
            Ok(())
        },
        &check,
    )
    .unwrap();
    assert_eq!(packed["packed"], 3);
    assert_eq!(packed["usage"]["packs"], 3);
    assert_eq!(packed["usage"]["descriptors"]["used"], 0);
    assert!(loose(&fixture).is_empty());
    drop(inputs);
    let mut inputs = Inputs::open(&options).unwrap();
    for index in 0..3 {
        assert!(
            inputs
                .broker
                .mutation_replay_recorded(&note(index), &metadata())
                .unwrap()
                .is_some()
        );
    }
    // 还原：先摆出还原后的条数；没确认不动，确认了逐字节回到原样。
    let options = fixture.options(Command::ArchiveUnpack);
    assert!(matches!(
        execute(
            &mut inputs,
            &options,
            actor(),
            |_, _| Err(MaintenanceError::Confirmation),
            check
        ),
        Err(MaintenanceError::Confirmation)
    ));
    assert!(loose(&fixture).is_empty());
    let restored = execute(
        &mut inputs,
        &options,
        actor(),
        |review, expected| {
            assert_eq!(expected, "archive-unpack");
            assert_eq!(review["after"]["descriptors"], 3);
            assert_eq!(review["after"]["over_limit"], false);
            Ok(())
        },
        check,
    )
    .unwrap();
    assert_eq!(restored["restored"]["packs"], 3);
    assert_eq!(loose(&fixture), before);
    assert!(!fixture.root.join("data/archive-v1").exists());
}

/// 有没投递完的审计：两条命令都拒，不进确认、不动东西。
#[test]
fn pending_audit_refuses_both_commands_before_confirmation() {
    let fixture = Fixture::new(true, true);
    let mut inputs = written(&fixture);
    sealed(&fixture.root.join("audit"));
    inputs.broker.mutate_recorded(note(9), metadata()).unwrap();
    let before = loose(&fixture);
    assert!(matches!(
        execute(
            &mut inputs,
            &fixture.options(Command::ArchiveUnpack),
            actor(),
            |_, _| panic!("有在途的事不进确认"),
            check
        ),
        Err(MaintenanceError::ArchiveBusy)
    ));
    assert!(matches!(
        archive::pack_at(&mut inputs, &actor(), later(), &mut accept, &check),
        Err(MaintenanceError::ArchiveBusy)
    ));
    assert_eq!(loose(&fixture), before);
}

/// 账本打不开：打包拒绝（没有水位可算），还原照样能做——退回旧程序不该被账本挡住。
#[test]
fn a_missing_ledger_refuses_packing_but_not_unpacking() {
    let fixture = Fixture::new(true, true);
    let mut inputs = written(&fixture);
    sealed(&fixture.root.join("audit"));
    let before = loose(&fixture);
    archive::pack_at(&mut inputs, &actor(), later(), &mut accept, &check).unwrap();
    inputs.audit = None;
    assert!(matches!(
        execute(
            &mut inputs,
            &fixture.options(Command::ArchivePack),
            actor(),
            |_, _| panic!("账本缺失不进确认"),
            check
        ),
        Err(MaintenanceError::Audit)
    ));
    execute(
        &mut inputs,
        &fixture.options(Command::ArchiveUnpack),
        actor(),
        accept,
        check,
    )
    .unwrap();
    assert_eq!(loose(&fixture), before);
}
