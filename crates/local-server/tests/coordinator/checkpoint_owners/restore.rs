//! 新根恢复接真实 owner（T-212）：人侧三件放置、重绑定、核链尾、Auth 撤权一整轮，启动闸门跟着数据走；
//! 接手要接在同一份恢复记录上，重绑定前接手逐字节重核；六 owner 封存里黑板三件各按自己的协议失效。
use super::{producer::Stage, seal::manifest, *};
use crate::checkpoint::{
    CheckpointTimes, NewRootRestore, RestoreRefusal, RevocationPending, require_restore_revoked,
    restore_into_new_root, trusted_catalog,
};
use hmac::{Hmac, Mac};
use std::path::{Path, PathBuf};
use studiolo_audit_ledger::AuditFilter;
use studiolo_auth::{SessionConfig, Totp, TotpCode, TotpSeed};
use studiolo_backup::{
    checkpoint::staging::{RestoredCheckpoint, SealedCheckpoint, SealedReference, StagingArea},
    recovery::{
        kit::{BootstrapChannel, PreRestoreChecks, RecoveryGrant, RecoveryIdentity, RecoveryKit},
        transcript::RecoveryStep,
    },
    worker::audit::{AdmissionFact, QueueAnswer, QueueCommand},
};
use studiolo_control_db::ControlDb;
use studiolo_core::checkpoint::InstantMillis;
use studiolo_policy::RevocationLevel;
use studiolo_vault_broker::FsVaultBroker;

pub(super) fn wall() -> i64 {
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
/// 虚构的主人 TOTP 种子；引用名和字节都不是任何真实秘密。
const SEED_REF: &str = "restore-drill-v1";
const SECRET: [u8; 20] = *b"restore-drill-secret";
fn seed(secret: [u8; 20]) -> TotpSeed {
    TotpSeed::new(SEED_REF.into(), secret).unwrap()
}
/// 给主人登记虚构的 TOTP：撤权要先挂上主人登记过的因子。
pub(super) fn enroll(sessions: &Arc<SessionManager>) {
    let (_totp, enrollment) = Totp::attach(sessions.clone(), seed(SECRET)).unwrap();
    enrollment
        .enroll(TotpCode::parse(otp(wall())).unwrap())
        .unwrap();
}
fn otp(now: i64) -> String {
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&SECRET).unwrap();
    mac.update(&((now / 30_000) as u64).to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 15);
    let value = u32::from_be_bytes(hash[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}
fn sessions(fixture: &Fixture) -> SessionConfig {
    Config::load(&fixture.config).unwrap().sessions().unwrap()
}
fn request<'a>(
    restored: &'a RestoredCheckpoint,
    new_root: &'a Path,
    complete: bool,
    sessions: SessionConfig,
) -> NewRootRestore<'a> {
    NewRootRestore {
        restored,
        new_root,
        restore_id: "drill-1",
        parent_lineage: None,
        resume_interrupted: false,
        checks: checks(complete),
        deadline: deadline(),
        sessions,
        totp_seed: seed(SECRET),
    }
}
/// 按私有配置打开新根上的 Auth，问启动闸门放不放行。
fn gate(fixture: &Fixture, new_root: &Path) -> Result<(), RevocationPending> {
    let ledger = Arc::new(SqliteAuditLedger::open_existing(&new_root.join("audit")).unwrap());
    let manager = SessionManager::open_existing(
        &new_root.join("auth"),
        sessions(fixture),
        ledger,
        Arc::new(SystemSessionClock),
        Arc::new(OsSessionRandom),
    )
    .unwrap();
    require_restore_revoked(Some(&new_root.join("data")), &manager)
}

/// 只装人侧三件的真实实例（主人登记了 TOTP），封存一次，返回夹具、原账本、封存和封存前的身份版本。
async fn human_only() -> (Fixture, Arc<SqliteAuditLedger>, SealedCheckpoint, i64) {
    let fixture = Fixture::new();
    let (mut broker, human) = source(&fixture);
    flush_content(&mut broker, &human, 0);
    let workbench = start(&fixture, broker, human.clone()).await;
    let config = Config::load(&fixture.config).unwrap();
    // 撤权只认 v4（恢复码存储）：新目录经 v3→v4 的迁移入口一次建到 v4。
    let sessions = Arc::new(
        SessionManager::migrate_v3(
            &fixture.root.join("auth"),
            config.sessions().unwrap(),
            human.clone(),
            Arc::new(SystemSessionClock),
            Arc::new(OsSessionRandom),
        )
        .unwrap(),
    );
    // 撤权要先挂上主人登记过的因子，所以封存之前登记一个虚构的 TOTP。
    enroll(&sessions);
    let epoch = sessions.identity_epoch().unwrap();
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
    (fixture, human, sealed, epoch)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn human_side_checkpoint_restores_with_auth_revocation_and_the_gate_follows_the_data() {
    let (fixture, _human, sealed, old_epoch) = human_only().await;
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
        restore_into_new_root(
            &mut held,
            request(&restored, &new_root, false, sessions(&fixture))
        )
        .unwrap_err(),
        RestoreRefusal::ChecksIncomplete
    );
    let mut foreign = grant(&SealedReference {
        checkpoint_id: CheckpointId::parse("another").unwrap(),
        seal_sha256: sealed.reference().seal_sha256.clone(),
    });
    assert_eq!(
        restore_into_new_root(
            &mut foreign,
            request(&restored, &new_root, true, sessions(&fixture))
        )
        .unwrap_err(),
        RestoreRefusal::Transcript
    );
    assert_eq!(fs::read_dir(&new_root).unwrap().count(), 0);
    assert_eq!(steps(&held), [RecoveryStep::KitOpened]);

    // 上一次放置中途失败留下的 `.restoring`：写任何东西之前就拒，声明接手也一样；人清掉才往下走（D2-F3）。
    fs::create_dir(new_root.join("auth.restoring")).unwrap();
    for resume in [false, true] {
        let mut leftover = request(&restored, &new_root, true, sessions(&fixture));
        leftover.resume_interrupted = resume;
        assert_eq!(
            restore_into_new_root(&mut held, leftover).unwrap_err(),
            RestoreRefusal::CleanupRequired
        );
    }
    assert_eq!(fs::read_dir(&new_root).unwrap().count(), 1);
    assert_eq!(steps(&held), [RecoveryStep::KitOpened]);
    fs::remove_dir(new_root.join("auth.restoring")).unwrap();

    // 拿错的 TOTP 种子：放置、重绑定都做完了，撤权那一步拒绝。新根放好了，但启动闸门不放行。
    let mut wrong = request(&restored, &new_root, true, sessions(&fixture));
    wrong.totp_seed = seed([7; 20]);
    assert_eq!(
        restore_into_new_root(&mut held, wrong).unwrap_err(),
        RestoreRefusal::Auth
    );
    let rebound = [
        RecoveryStep::KitOpened,
        RecoveryStep::PreRestoreChecksCompleted,
        RecoveryStep::PhysicalRebindIntent,
        RecoveryStep::PhysicalRebindOutcome,
        RecoveryStep::OwnerRestoreIntent,
    ];
    assert_eq!(steps(&held), rebound);
    let mut names: Vec<_> = fs::read_dir(&new_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["audit", "auth", "data", "vault"]);
    assert_eq!(gate(&fixture, &new_root), Err(RevocationPending));

    // 同一新根、没声明接手：拒绝，不覆盖。
    let mut again = grant(sealed.reference());
    assert_eq!(
        restore_into_new_root(
            &mut again,
            request(&restored, &new_root, true, sessions(&fixture))
        )
        .unwrap_err(),
        RestoreRefusal::AlreadyPlaced
    );
    assert_eq!(steps(&again), [RecoveryStep::KitOpened]);
    // 声明接手却拿一份新开的记录：接不上上一次，拒绝；拿用过的记录去另一个空新根新开一次，也拒绝（D2-F2）。
    let mut resume = request(&restored, &new_root, true, sessions(&fixture));
    resume.resume_interrupted = true;
    assert_eq!(
        restore_into_new_root(&mut again, resume).unwrap_err(),
        RestoreRefusal::Transcript
    );
    assert_eq!(steps(&again), [RecoveryStep::KitOpened]);
    let elsewhere = target(&fixture, "elsewhere");
    assert_eq!(
        restore_into_new_root(
            &mut held,
            request(&restored, &elsewhere, true, sessions(&fixture))
        )
        .unwrap_err(),
        RestoreRefusal::Transcript
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    assert_eq!(steps(&held), rebound);

    // 人看过之后在同一份记录上声明接手、拿对的种子：不再重放置、不再重绑，Auth 已经开始过，接着做完。
    let mut resume = request(&restored, &new_root, true, sessions(&fixture));
    resume.resume_interrupted = true;
    let report = restore_into_new_root(&mut held, resume).unwrap();
    assert!(report.reused_placement);
    assert!(report.invalidation.audit_confirmed);
    assert!(report.invalidation.epoch > old_epoch);
    assert_eq!(report.lineage.restore_id.as_deref(), Some("drill-1"));
    assert_eq!(
        report.restored_owners,
        [Owner::Broker, Owner::Auth, Owner::Audit]
    );
    assert!(!report.sync_left_closed);
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
    assert_eq!(report.lineage.vault_uuid, vault_uuid);
    assert_eq!(report.lineage.pair_id, pair_id);
    assert_eq!(
        steps(&held)[rebound.len()..],
        [
            RecoveryStep::PreRestoreChecksCompleted,
            RecoveryStep::PhysicalRebindIntent,
            RecoveryStep::PhysicalRebindOutcome,
            RecoveryStep::AuthInvalidated,
        ]
    );
    held.transcript.verify().unwrap();
    FsVaultBroker::open_existing(&new_root.join("vault"), &new_root.join("data")).unwrap();
    // 人侧账本在水位之后正好多出撤权自己记的两条恢复事件。
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
        .query(&AuditFilter::default().after_offset(last_offset))
        .unwrap();
    assert_eq!(tail.len(), 2);
    assert!(
        tail.iter()
            .all(|entry| entry.event().action() == "session.revoked")
    );
    drop(ledger);

    // 启动闸门：同一恢复标识撤权做完才放行；lineage 换成别的恢复标识、或第 1 版没有标识，都不放行。
    assert_eq!(gate(&fixture, &new_root), Ok(()));
    let lineage = new_root.join("data/restore-v1/lineage.json");
    let original = fs::read(&lineage).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["restore_id"] = "drill-2".into();
    fs::write(&lineage, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(gate(&fixture, &new_root), Err(RevocationPending));
    value["format_version"] = 1.into();
    value.as_object_mut().unwrap().remove("restore_id");
    fs::write(&lineage, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(gate(&fixture, &new_root), Err(RevocationPending));
    fs::write(&lineage, &original).unwrap();
    assert_eq!(gate(&fixture, &new_root), Ok(()));

    // 再接手一次是幂等的：同一份记录、同一恢复标识，身份版本不变，`AuthInvalidated` 不再记第二次。
    let before = held.transcript.entries.len();
    let mut resume = request(&restored, &new_root, true, sessions(&fixture));
    resume.resume_interrupted = true;
    let again = restore_into_new_root(&mut held, resume).unwrap();
    assert_eq!(again.invalidation.epoch, report.invalidation.epoch);
    assert_eq!(
        steps(&held)[before..],
        [
            RecoveryStep::PreRestoreChecksCompleted,
            RecoveryStep::PhysicalRebindIntent,
            RecoveryStep::PhysicalRebindOutcome,
        ]
    );

    // 新根的人侧账本在水位之后多了一条别的事件：接手时链尾核对拒绝（D2-F2 ④）。
    let ledger = SqliteAuditLedger::open_existing(&new_root.join("audit")).unwrap();
    ledger
        .append(studiolo_audit_ledger::AuditEvent::new(
            studiolo_audit_ledger::AuditEventId::parse("restore-test:foreign").unwrap(),
            studiolo_audit_ledger::AuditPhase::Intent,
            "task.reconcile",
            studiolo_audit_ledger::SanitizedPayload::try_from(
                serde_json::json!({"request_hash": "0".repeat(64)}),
            )
            .unwrap(),
        ))
        .unwrap();
    drop(ledger);
    let mut resume = request(&restored, &new_root, true, sessions(&fixture));
    resume.resume_interrupted = true;
    assert_eq!(
        restore_into_new_root(&mut held, resume).unwrap_err(),
        RestoreRefusal::Audit
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resuming_before_the_rebind_rechecks_every_placed_component() {
    let (fixture, _human, sealed, _) = human_only().await;
    let fetched = target(&fixture, "fetched").join("copy");
    fetch(&sealed.ticket().transfer_source.root, &fetched);
    let restored = RestoredCheckpoint::open(&fetched, sealed.reference(), deadline()).unwrap();
    let new_root = target(&fixture, "new-root");
    // 上一次记下了放置意图、四个组件都放好了，还没开始 Broker 重绑定就停了。
    let mut held = grant(sealed.reference());
    held.transcript
        .append(
            RecoveryStep::PhysicalRebindIntent,
            serde_json::json!({"restore": "drill-1", "reused_components": 0, "resumed": false}),
        )
        .unwrap();
    for (component, name) in [
        ("canonical", "vault"),
        ("writer-data", "data"),
        ("audit", "audit"),
        ("auth", "auth"),
    ] {
        restored
            .extract(component, &new_root, name, deadline())
            .unwrap();
    }
    let resume = || {
        let mut request = request(&restored, &new_root, true, sessions(&fixture));
        request.resume_interrupted = true;
        request
    };
    // 改了一个字节、多了一个文件：都在写任何东西之前拒绝，恢复记录不多一行。
    let file = new_root.join("auth/auth.sqlite3");
    let original = fs::read(&file).unwrap();
    let mut changed = original.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fs::write(&file, &changed).unwrap();
    assert_eq!(
        restore_into_new_root(&mut held, resume()).unwrap_err(),
        RestoreRefusal::Placement
    );
    fs::write(&file, &original).unwrap();
    let extra = new_root.join("vault/extra.md");
    fs::write(&extra, b"extra").unwrap();
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        restore_into_new_root(&mut held, resume()).unwrap_err(),
        RestoreRefusal::Placement
    );
    fs::remove_file(&extra).unwrap();
    assert_eq!(
        steps(&held),
        [RecoveryStep::KitOpened, RecoveryStep::PhysicalRebindIntent]
    );
    // 都复原之后接手：重核通过，接着重绑定、核链尾、撤权，做完；启动闸门放行。
    let report = restore_into_new_root(&mut held, resume()).unwrap();
    assert!(report.reused_placement);
    assert!(report.invalidation.audit_confirmed);
    assert_eq!(gate(&fixture, &new_root), Ok(()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn six_owner_checkpoint_restores_and_invalidates_every_board_owner() {
    // 真实六 owner 经生产者封存：Control、AgentStore、Policy 各按自己的协议失效，Auth 撤权最后做。
    let stage = Stage::with_owner_factor().await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    let QueueAnswer::Pending(record) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };
    let fact = AdmissionFact::from_receipt(&record).unwrap();
    let source: PathBuf = SealedCheckpoint::root_path(&stage.parent, &fact.key().checkpoint_id);
    let fixture = &stage.assembled.fixture;
    let fetched = target(fixture, "fetched").join("copy");
    fetch(&source, &fetched);
    let restored = RestoredCheckpoint::open(&fetched, fact.source(), deadline()).unwrap();
    let new_root = target(fixture, "new-root");
    let mut held = grant(fact.source());
    let report = restore_into_new_root(
        &mut held,
        request(&restored, &new_root, true, sessions(fixture)),
    )
    .unwrap();
    assert_eq!(
        report.restored_owners,
        [
            Owner::Broker,
            Owner::Control,
            Owner::Auth,
            Owner::AgentStore,
            Owner::Policy,
            Owner::Audit
        ]
    );
    assert!(!report.sync_left_closed);
    assert!(report.invalidation.audit_confirmed);
    assert_eq!(
        steps(&held),
        [
            RecoveryStep::KitOpened,
            RecoveryStep::PreRestoreChecksCompleted,
            RecoveryStep::PhysicalRebindIntent,
            RecoveryStep::PhysicalRebindOutcome,
            RecoveryStep::OwnerRestoreIntent,
            RecoveryStep::ControlLeasesRevoked,
            RecoveryStep::OwnerRestoreIntent,
            RecoveryStep::PolicyGloballyRevoked,
            RecoveryStep::OwnerRestoreIntent,
            RecoveryStep::AgentCredentialsInvalidated,
            RecoveryStep::OwnerRestoreIntent,
            RecoveryStep::AuthInvalidated,
        ]
    );
    held.transcript.verify().unwrap();
    let mut names: Vec<_> = fs::read_dir(&new_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "agent", "audit", "auth", "control", "data", "policy", "vault"
        ]
    );
    // 旧类型 token 全部失效；登记过的实例在策略库是吊销状态；策略库全局吊销，只能经 L3 解除。
    let agents = AgentStore::open_existing(&new_root.join("agent")).unwrap();
    assert!(
        agents
            .types()
            .unwrap()
            .iter()
            .all(|(_, _, revoked)| *revoked)
    );
    let instances = agents.instances(10).unwrap();
    assert_eq!(instances.len(), 1);
    drop(agents);
    let policy = PolicyStore::open_existing(&new_root.join("policy")).unwrap();
    assert!(policy.effective().unwrap().global_revoked);
    assert!(policy.revocations(true).unwrap().iter().any(|revocation| {
        revocation.level == RevocationLevel::Instance && revocation.target == instances[0].0.id
    }));
    drop(policy);
    // Control：放出来的库只开不建，拿着租约的 attempt 一个都没有。
    let OwnerWatermark::Broker { vault_uuid, .. } = restored
        .manifest()
        .watermarks
        .iter()
        .find(|mark| matches!(mark, OwnerWatermark::Broker { .. }))
        .unwrap()
        .clone()
    else {
        unreachable!()
    };
    let control = ControlDb::open_existing(&new_root.join("control"), vault_uuid).unwrap();
    assert_eq!(control.live_attempts().unwrap().leased, 0);
    drop(control);
    assert_eq!(gate(fixture, &new_root), Ok(()));
    stage.assembled.workbench.shutdown().await.unwrap();
}
