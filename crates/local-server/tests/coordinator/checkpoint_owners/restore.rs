//! 新根恢复接真实 owner：只有人侧三件（Broker、Auth、Audit）的封存能恢复；带黑板的在写之前拒绝。
use super::{producer::Stage, seal::manifest, *};
use crate::checkpoint::{
    CheckpointTimes, NewRootRestore, RestoreRefusal, restore_into_new_root, trusted_catalog,
};
use std::path::{Path, PathBuf};
use studiolo_audit_ledger::AuditFilter;
use studiolo_backup::{
    checkpoint::staging::{RestoredCheckpoint, SealedCheckpoint, SealedReference, StagingArea},
    recovery::{
        kit::{BootstrapChannel, PreRestoreChecks, RecoveryGrant, RecoveryIdentity, RecoveryKit},
        transcript::RecoveryStep,
    },
    worker::audit::{AdmissionFact, QueueAnswer, QueueCommand},
};
use studiolo_core::checkpoint::InstantMillis;
use studiolo_vault_broker::FsVaultBroker;

fn wall() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
/// 模拟「从远端取回」：整棵复制到别处，目录 0500、文件 0400。
fn fetch(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fetch(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o400)).unwrap();
        }
    }
    fs::set_permissions(to, fs::Permissions::from_mode(0o500)).unwrap();
}
fn grant(reference: &SealedReference) -> RecoveryGrant {
    RecoveryKit {
        checkpoint_id: reference.checkpoint_id.clone(),
        identity: RecoveryIdentity {
            identity_id: "drill".into(),
            expires_at: InstantMillis::from_millis(i64::MAX),
            redeemed: false,
        },
        package_digest: "ab".repeat(32),
        secret_references: Vec::new(),
        sync_recovery_material: false,
        stored_outside_ordinary_checkpoint: true,
    }
    .redeem(BootstrapChannel::LocalTty, InstantMillis::from_millis(0))
    .unwrap()
}
fn checks(complete: bool) -> PreRestoreChecks {
    PreRestoreChecks {
        sync_adapter_disabled: true,
        execution_adapter_disabled: true,
        content_verified: true,
        version_verified: true,
        permissions_verified: true,
        hashes_verified: true,
        audit_continuity_verified: true,
        space_verified: complete,
    }
}
fn steps(grant: &RecoveryGrant) -> Vec<RecoveryStep> {
    grant
        .transcript
        .entries
        .iter()
        .map(|entry| entry.step)
        .collect()
}
fn request<'a>(
    restored: &'a RestoredCheckpoint,
    new_root: &'a Path,
    complete: bool,
) -> NewRootRestore<'a> {
    NewRootRestore {
        restored,
        new_root,
        restore_id: "drill-1",
        parent_lineage: None,
        resume_interrupted: false,
        checks: checks(complete),
        deadline: deadline(),
    }
}

/// 只装人侧三件的真实实例，封存一次，返回夹具、原账本和封存。
async fn human_only() -> (Fixture, Arc<SqliteAuditLedger>, SealedCheckpoint) {
    let fixture = Fixture::new();
    let (mut broker, human) = source(&fixture);
    flush_content(&mut broker, &human, 0);
    let workbench = start(&fixture, broker, human.clone()).await;
    let config = Config::load(&fixture.config).unwrap();
    let sessions = Arc::new(
        SessionManager::open(
            &fixture.root.join("auth"),
            config.sessions().unwrap(),
            human.clone(),
            Arc::new(SystemSessionClock),
            Arc::new(OsSessionRandom),
        )
        .unwrap(),
    );
    let inputs = CheckpointInputs {
        workbench,
        sessions,
        audit: human.clone(),
        board: None,
        bridge: None,
        policy: None,
    };
    let key = JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("human-only").unwrap(),
    };
    human
        .append(backup_intent(&key, BackupTargetKind::LocalTemporaryRepository).unwrap())
        .unwrap();
    let parent = target(&fixture, "staging");
    let ephemeral = target(&fixture, "ephemeral");
    let sources = ["vault", "data", "auth", "audit"].map(|name| fixture.root.join(name));
    let area = StagingArea::create(
        &parent,
        key.checkpoint_id.clone(),
        &sources,
        &ephemeral,
        Instant::now() + Duration::from_secs(25),
    )
    .unwrap();
    let frozen = inputs
        .bind()
        .unwrap()
        .freeze(
            FreezeRequest {
                checkpoint_id: key.checkpoint_id.clone(),
                budget: FreezeBudget::new(30_000).unwrap(),
            },
            &key,
            BackupTargetKind::LocalTemporaryRepository,
            OwnerExportPaths::under(area.export_root()),
        )
        .unwrap();
    let now = wall();
    let sealed = frozen
        .seal(
            area,
            trusted_catalog,
            manifest(),
            CheckpointTimes {
                content_cutoff: InstantMillis::from_millis(now - 100),
                now: InstantMillis::from_millis(now),
                expires_at: InstantMillis::from_millis(now + 30_000),
            },
        )
        .unwrap();
    drop(frozen);
    (fixture, human, sealed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_side_checkpoint_restores_into_a_new_root_and_rebinds_the_broker() {
    let (fixture, _human, sealed) = human_only().await;
    assert_eq!(
        sealed.manifest().roster.assembly(Owner::Control),
        OwnerAssembly::Absent
    );
    let fetched = target(&fixture, "fetched").join("copy");
    fetch(&sealed.ticket().transfer_source.root, &fetched);
    let restored = RestoredCheckpoint::open(&fetched, sealed.reference(), deadline()).unwrap();
    let new_root = target(&fixture, "new-root");

    // 前置检查没确认全、恢复记录不是这个 checkpoint 的：都在写之前拒绝，记录里也不多一行。
    let mut held = grant(sealed.reference());
    assert_eq!(
        restore_into_new_root(&mut held, request(&restored, &new_root, false)).unwrap_err(),
        RestoreRefusal::ChecksIncomplete
    );
    let mut foreign = grant(&SealedReference {
        checkpoint_id: CheckpointId::parse("another").unwrap(),
        seal_sha256: sealed.reference().seal_sha256.clone(),
    });
    assert_eq!(
        restore_into_new_root(&mut foreign, request(&restored, &new_root, true)).unwrap_err(),
        RestoreRefusal::Transcript
    );
    assert_eq!(fs::read_dir(&new_root).unwrap().count(), 0);
    assert_eq!(steps(&held), [RecoveryStep::KitOpened]);

    let report = restore_into_new_root(&mut held, request(&restored, &new_root, true)).unwrap();
    assert!(!report.reused_placement);
    let OwnerWatermark::Broker {
        vault_uuid,
        pair_id,
        ..
    } = sealed
        .manifest()
        .watermarks
        .iter()
        .find(|mark| matches!(mark, OwnerWatermark::Broker { .. }))
        .unwrap()
        .clone()
    else {
        unreachable!()
    };
    assert_eq!(report.receipt.vault_uuid, vault_uuid);
    assert_eq!(report.receipt.pair_id, pair_id);
    assert_eq!(
        steps(&held),
        [
            RecoveryStep::KitOpened,
            RecoveryStep::PreRestoreChecksCompleted,
            RecoveryStep::PhysicalRebindIntent,
            RecoveryStep::PhysicalRebindOutcome,
        ]
    );
    held.transcript.verify().unwrap();

    // 新根上的四个组件都在；Broker 用普通的「只开已有库」入口就能开，说明物理绑定已经换成新目录。
    let mut names: Vec<_> = fs::read_dir(&new_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["audit", "auth", "data", "vault"]);
    FsVaultBroker::open_existing(&new_root.join("vault"), &new_root.join("data")).unwrap();
    // 人侧账本的链尾正好是清单里的水位；Auth 的库只放置，这里没有打开过它。
    let OwnerWatermark::Audit { last_offset, .. } = sealed
        .manifest()
        .watermarks
        .iter()
        .find(|mark| matches!(mark, OwnerWatermark::Audit { .. }))
        .unwrap()
        .clone()
    else {
        unreachable!()
    };
    let ledger = SqliteAuditLedger::open_existing(&new_root.join("audit")).unwrap();
    let tail = ledger
        .query(&AuditFilter::default().after_offset(last_offset - 1))
        .unwrap();
    assert_eq!(tail.len(), 1);
    assert!(new_root.join("auth/auth.sqlite3").is_file());

    // 同一个新根再来一次、又没声明接手：拒绝，不覆盖。
    let mut again = grant(sealed.reference());
    assert_eq!(
        restore_into_new_root(&mut again, request(&restored, &new_root, true)).unwrap_err(),
        RestoreRefusal::AlreadyPlaced
    );
    assert_eq!(steps(&again), [RecoveryStep::KitOpened]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoint_with_board_owners_is_refused_before_anything_is_written() {
    // 真实六 owner 经生产者封存：名单里有 Control、AgentStore、Policy，它们没有新根恢复协议。
    let stage = Stage::new(true).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    let QueueAnswer::Pending(record) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };
    let fact = AdmissionFact::from_receipt(&record).unwrap();
    let source: PathBuf = SealedCheckpoint::root_path(&stage.parent, &fact.key().checkpoint_id);
    let fetched = target(&stage.assembled.fixture, "fetched").join("copy");
    fetch(&source, &fetched);
    // 取回副本的只读核验对六 owner 的封存同样成立；拒绝发生在恢复编排那一层。
    let restored = RestoredCheckpoint::open(&fetched, fact.source(), deadline()).unwrap();
    let new_root = target(&stage.assembled.fixture, "new-root");
    let mut held = grant(fact.source());
    assert_eq!(
        restore_into_new_root(&mut held, request(&restored, &new_root, true)).unwrap_err(),
        RestoreRefusal::Unsupported(Owner::Control)
    );
    assert_eq!(fs::read_dir(&new_root).unwrap().count(), 0);
    assert_eq!(steps(&held), [RecoveryStep::KitOpened]);
}
