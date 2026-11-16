//! 新根恢复接真实 owner（T-212）：人侧三件放置、重绑定、核链尾、Auth 撤权一整轮，启动闸门跟着数据走；
//! 接手要接在同一份恢复记录上，重绑定前接手逐字节重核；六 owner 封存里黑板三件各按自己的协议失效。
//! 命令行的写法（每追加一条先交出去）中途交不出去时，拿交出去的文本接手；最终切换两步要主人重新登录。
//! Auth、身份库撤权做完之前在暂用名下，按正式路径打不开；被封存的根本身是恢复出来的，它的 lineage 只当上一环，
//! 用过的恢复标识再用一律拒绝（Codex 第二意见第 1、2、5 条）。
use super::{producer::Stage, seal::manifest, *};
use crate::checkpoint::{
    CheckpointTimes, Cutover, CutoverRefusal, NewRootRestore, RestoreRefusal, RestoreReport,
    RevocationPending, begin_cutover, finish_cutover, require_restore_revoked,
    restore_into_new_root, restore_started, trusted_catalog,
};
use hmac::{Hmac, Mac};
use std::path::{Path, PathBuf};
use studiolo_audit_ledger::AuditFilter;
use studiolo_auth::{Recovery, SessionConfig, Totp, TotpCode, TotpSeed};
use studiolo_backup::{
    checkpoint::staging::{RestoredCheckpoint, SealedCheckpoint, SealedReference, StagingArea},
    recovery::{
        kit::{BootstrapChannel, PreRestoreChecks, RecoveryGrant, RecoveryIdentity, RecoveryKit},
        transcript::{RecoveryStep, RecoveryTranscript, TranscriptEntry},
    },
    worker::audit::{AdmissionFact, QueueAnswer, QueueCommand},
};
use studiolo_control_db::ControlDb;
use studiolo_core::{TaskId, checkpoint::InstantMillis};
use studiolo_policy::RevocationLevel;
use studiolo_vault_broker::{FsVaultBroker, read_restore_lineage};

/// Access 测试夹具（公开的测试密钥）：模拟主人在新根上重新登录。
#[path = "../../access/test_support.rs"]
mod access_fixture;

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
        resume_interrupted: false,
        checks: checks(complete),
        deadline: deadline(),
        sessions,
        totp_seed: seed(SECRET),
    }
}
/// 同 [`request`]，换一个恢复标识。
fn request_as<'a>(
    restored: &'a RestoredCheckpoint,
    new_root: &'a Path,
    sessions: SessionConfig,
    restore_id: &'a str,
) -> NewRootRestore<'a> {
    NewRootRestore {
        restore_id,
        ..request(restored, new_root, true, sessions)
    }
}
/// 进程内恢复：记录只留在 `grant` 里，不另交给别处。
fn restore(
    grant: &mut RecoveryGrant,
    request: NewRootRestore<'_>,
) -> Result<RestoreReport, RestoreRefusal> {
    restore_into_new_root(grant, request, &mut |_| Ok(()))
}
/// 命令行的写法：已有的整份先写出去（头一行加每条一行）。
fn text_of(transcript: &RecoveryTranscript) -> Vec<String> {
    let mut lines = vec![transcript.header_line().unwrap()];
    lines.extend(
        transcript
            .entries
            .iter()
            .map(|entry| RecoveryTranscript::entry_line(entry).unwrap()),
    );
    lines
}
/// 命令行的去处：每追加一条写一行。
fn written(lines: &mut Vec<String>) -> impl FnMut(&TranscriptEntry) -> Result<(), ()> + '_ {
    move |entry: &TranscriptEntry| {
        lines.push(RecoveryTranscript::entry_line(entry).map_err(drop)?);
        Ok(())
    }
}
/// 接手或切换时读回人手里的那份文本。
fn read_back(lines: &[String]) -> RecoveryGrant {
    let transcript = RecoveryTranscript::from_lines(&lines.join("\n")).unwrap();
    RecoveryGrant {
        authorization: transcript.authorize_write().unwrap(),
        transcript,
    }
}
fn cutover<'a>(fixture: &Fixture, new_root: &'a Path) -> Cutover<'a> {
    Cutover {
        new_root,
        restore_id: "drill-1",
        sessions: sessions(fixture),
    }
}
/// 主人在新根上重新登录一次：像服务那样打开新根的 Auth，拿 Access 夹具签的令牌签发会话。
async fn login(fixture: &Fixture, new_root: &Path) {
    let identity = access_fixture::verifier()
        .await
        .verify(&access_fixture::signed(&access_fixture::claims()))
        .await
        .unwrap();
    let ledger = Arc::new(SqliteAuditLedger::open_existing(&new_root.join("audit")).unwrap());
    let manager = SessionManager::open_existing(
        &new_root.join("auth"),
        sessions(fixture),
        ledger,
        Arc::new(SystemSessionClock),
        Arc::new(OsSessionRandom),
    )
    .unwrap();
    manager.issue(&identity).unwrap();
}
/// 主服务按生产配置起在新根上会怎样。
#[derive(Debug, PartialEq, Eq)]
enum Startup {
    /// Auth 打得开、启动闸门放行。
    Serves,
    /// 那个路径下没有 Auth（撤权没做完时 Auth 在暂用名 `auth.held` 下）：服务起不来。
    AuthMissing,
    /// Auth 打得开，启动闸门不放行。
    Pending,
}
/// 按私有配置的正式路径打开新根上的 Auth，问启动闸门放不放行。
fn gate(fixture: &Fixture, new_root: &Path) -> Startup {
    gate_at(fixture, new_root, "auth")
}
/// 按指定的 Auth 目录名打开、问启动闸门：拿暂用名问，说明暂用名和启动闸门是两道。
fn gate_at(fixture: &Fixture, new_root: &Path, auth: &str) -> Startup {
    let directory = new_root.join(auth);
    if !directory.exists() {
        return Startup::AuthMissing;
    }
    let ledger = Arc::new(SqliteAuditLedger::open_existing(&new_root.join("audit")).unwrap());
    let manager = SessionManager::open_existing(
        &directory,
        sessions(fixture),
        ledger,
        Arc::new(SystemSessionClock),
        Arc::new(OsSessionRandom),
    )
    .unwrap();
    match require_restore_revoked(Some(&new_root.join("data")), &manager) {
        Ok(()) => Startup::Serves,
        Err(RevocationPending) => Startup::Pending,
    }
}
fn names(new_root: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(new_root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}
/// 在源 data 里放一份「这个根自己是恢复出来的」已完成 lineage（恢复标识 drill-0、上一个 checkpoint），
/// Broker 的封存把 data 整棵带走，和真的恢复出来的根一样。
fn ancestor_lineage(data: &Path) {
    let binding: serde_json::Value =
        serde_json::from_slice(&fs::read(data.join("broker-binding.json")).unwrap()).unwrap();
    let id = |ino: u64| serde_json::json!({ "dev": 1, "ino": ino });
    let lineage = serde_json::json!({
        "format_version": 3,
        "state": "completed",
        "vault_uuid": binding["vault_uuid"],
        "pair_id": binding["pair_id"],
        "checkpoint_id": "an-older-checkpoint",
        "previous_canonical": id(1),
        "previous_data": id(2),
        "restored_canonical": id(3),
        "restored_data": id(4),
        "parent_lineage": null,
        "restore_id": "drill-0",
    });
    let directory = data.join("restore-v1");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let file = directory.join("lineage.json");
    fs::write(&file, serde_json::to_vec(&lineage).unwrap()).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
}

/// 只装人侧三件的真实实例（主人登记了 TOTP），封存一次，返回夹具、原账本、封存和封存前的身份版本。
async fn human_only() -> (Fixture, Arc<SqliteAuditLedger>, SealedCheckpoint, i64) {
    seal_human(false).await
}
/// 同上；`already_restored` 时这个根本身是恢复出来的：data 里留着恢复标识 drill-0 的已完成 lineage，Auth 里
/// 留着 drill-0 的撤权记录，另有一条 lineage 那一串里没有的 drill-9（测 Auth 自己那一道）。
async fn seal_human(
    already_restored: bool,
) -> (Fixture, Arc<SqliteAuditLedger>, SealedCheckpoint, i64) {
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
    if already_restored {
        let (_totp, _enrollment) = Totp::attach(sessions.clone(), seed(SECRET)).unwrap();
        let _recovery = Recovery::attach(sessions.clone()).unwrap();
        for earlier in ["drill-0", "drill-9"] {
            let report = sessions.invalidate_restored_credentials(earlier).unwrap();
            assert!(report.audit_confirmed);
        }
        ancestor_lineage(&fixture.root.join("data"));
    }
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

    // 恢复标识不合法、前置检查没确认全、恢复记录不是这个 checkpoint 的：都在写之前拒绝，记录里也不多一行。
    let mut held = grant(sealed.reference());
    let mut bad = request(&restored, &new_root, true, sessions(&fixture));
    bad.restore_id = "drill 1";
    assert_eq!(
        restore(&mut held, bad).unwrap_err(),
        RestoreRefusal::RestoreId
    );
    assert_eq!(
        restore(
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
        restore(
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
            restore(&mut held, leftover).unwrap_err(),
            RestoreRefusal::CleanupRequired
        );
    }
    assert_eq!(fs::read_dir(&new_root).unwrap().count(), 1);
    assert_eq!(steps(&held), [RecoveryStep::KitOpened]);
    fs::remove_dir(new_root.join("auth.restoring")).unwrap();
    // Auth 放在暂用名下，暂用名的半成品一样拒。
    fs::create_dir(new_root.join("auth.held.restoring")).unwrap();
    assert_eq!(
        restore(
            &mut held,
            request(&restored, &new_root, true, sessions(&fixture))
        )
        .unwrap_err(),
        RestoreRefusal::CleanupRequired
    );
    assert_eq!(steps(&held), [RecoveryStep::KitOpened]);
    fs::remove_dir(new_root.join("auth.held.restoring")).unwrap();

    // 拿错的 TOTP 种子：放置、重绑定都做完了，撤权那一步拒绝。新根放好了，Auth 还在暂用名下，按正式路径
    // 起服务根本打不开 Auth；拿暂用名去开，启动闸门也不放行（两道）。
    let mut wrong = request(&restored, &new_root, true, sessions(&fixture));
    wrong.totp_seed = seed([7; 20]);
    assert_eq!(restore(&mut held, wrong).unwrap_err(), RestoreRefusal::Auth);
    let rebound = [
        RecoveryStep::KitOpened,
        RecoveryStep::PreRestoreChecksCompleted,
        RecoveryStep::PhysicalRebindIntent,
        RecoveryStep::PhysicalRebindOutcome,
        RecoveryStep::OwnerRestoreIntent,
    ];
    assert_eq!(steps(&held), rebound);
    assert_eq!(names(&new_root), ["audit", "auth.held", "data", "vault"]);
    assert_eq!(gate(&fixture, &new_root), Startup::AuthMissing);
    assert_eq!(gate_at(&fixture, &new_root, "auth.held"), Startup::Pending);

    // 同一新根、没声明接手：拒绝，不覆盖。
    let mut again = grant(sealed.reference());
    assert_eq!(
        restore(
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
        restore(&mut again, resume).unwrap_err(),
        RestoreRefusal::Transcript
    );
    assert_eq!(steps(&again), [RecoveryStep::KitOpened]);
    let elsewhere = target(&fixture, "elsewhere");
    assert_eq!(
        restore(
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
    let report = restore(&mut held, resume).unwrap();
    assert!(report.reused_placement);
    assert!(report.invalidation.audit_confirmed);
    assert_eq!(report.invalidation.epoch, old_epoch + 1);
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
    // 撤权做完才改成正式名，暂用名不再留着。
    assert_eq!(names(&new_root), ["audit", "auth", "data", "vault"]);
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
    assert_eq!(gate(&fixture, &new_root), Startup::Serves);
    let lineage = new_root.join("data/restore-v1/lineage.json");
    let original = fs::read(&lineage).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["restore_id"] = "drill-2".into();
    fs::write(&lineage, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(gate(&fixture, &new_root), Startup::Pending);
    value["format_version"] = 1.into();
    value.as_object_mut().unwrap().remove("restore_id");
    fs::write(&lineage, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(gate(&fixture, &new_root), Startup::Pending);
    fs::write(&lineage, &original).unwrap();
    assert_eq!(gate(&fixture, &new_root), Startup::Serves);

    // 再接手一次是幂等的：同一份记录、同一恢复标识，身份版本不变，`AuthInvalidated` 不再记第二次。
    let before = held.transcript.entries.len();
    let mut resume = request(&restored, &new_root, true, sessions(&fixture));
    resume.resume_interrupted = true;
    let again = restore(&mut held, resume).unwrap();
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
        restore(&mut held, resume).unwrap_err(),
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
    // 上一次记下了放置意图、四个组件都放好了（Auth 在暂用名下），还没开始 Broker 重绑定就停了。
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
        ("auth", "auth.held"),
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
    // 重绑定之前 Auth 就出现在正式名下（暂用名只在它的撤权做完之后才改）：不认，拒绝；两个名字同时在也拒绝。
    fs::rename(new_root.join("auth.held"), new_root.join("auth")).unwrap();
    assert_eq!(
        restore(&mut held, resume()).unwrap_err(),
        RestoreRefusal::Placement
    );
    fs::create_dir(new_root.join("auth.held")).unwrap();
    assert_eq!(
        restore(&mut held, resume()).unwrap_err(),
        RestoreRefusal::Placement
    );
    fs::remove_dir(new_root.join("auth.held")).unwrap();
    fs::rename(new_root.join("auth"), new_root.join("auth.held")).unwrap();
    // 改了一个字节、多了一个文件：都在写任何东西之前拒绝，恢复记录不多一行。
    let file = new_root.join("auth.held/auth.sqlite3");
    let original = fs::read(&file).unwrap();
    let mut changed = original.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    fs::write(&file, &changed).unwrap();
    assert_eq!(
        restore(&mut held, resume()).unwrap_err(),
        RestoreRefusal::Placement
    );
    fs::write(&file, &original).unwrap();
    let extra = new_root.join("vault/extra.md");
    fs::write(&extra, b"extra").unwrap();
    fs::set_permissions(&extra, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        restore(&mut held, resume()).unwrap_err(),
        RestoreRefusal::Placement
    );
    fs::remove_file(&extra).unwrap();
    assert_eq!(
        steps(&held),
        [RecoveryStep::KitOpened, RecoveryStep::PhysicalRebindIntent]
    );
    // 都复原之后接手：重核通过，接着重绑定、核链尾、撤权，做完；启动闸门放行。
    let report = restore(&mut held, resume()).unwrap();
    assert!(report.reused_placement);
    assert!(report.invalidation.audit_confirmed);
    assert_eq!(gate(&fixture, &new_root), Startup::Serves);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stop_mid_journal_resumes_from_the_delivered_text_and_cutover_needs_a_fresh_login() {
    let (fixture, _human, sealed, old_epoch) = human_only().await;
    let fetched = target(&fixture, "fetched").join("copy");
    fetch(&sealed.ticket().transfer_source.root, &fetched);
    let restored = RestoredCheckpoint::open(&fetched, sealed.reference(), deadline()).unwrap();
    let new_root = target(&fixture, "new-root");

    // 命令行新开时的恢复包：从核验过的副本当场生成（封存摘要、清单里的秘密引用），马上兑换。
    let mut kit = RecoveryKit::for_restored(
        &restored,
        "restore-drill".into(),
        InstantMillis::from_millis(wall() + 600_000),
    );
    assert_eq!(kit.checkpoint_id, sealed.reference().checkpoint_id);
    assert_eq!(kit.package_digest, sealed.reference().seal_sha256);
    assert_eq!(kit.secret_references, restored.manifest().secret_references);
    assert!(kit.stored_outside_ordinary_checkpoint && !kit.sync_recovery_material);
    let mut held = kit
        .redeem(
            BootstrapChannel::LocalTty,
            InstantMillis::from_millis(wall()),
        )
        .unwrap();

    // 放置意图那条就交不出去：一个组件都没放，内存里也没有这次的放置意图，命令行据此说「这次没写过新根」。
    let mut untouched = RecoveryKit::for_restored(
        &restored,
        "restore-drill-0".into(),
        InstantMillis::from_millis(wall() + 600_000),
    )
    .redeem(
        BootstrapChannel::LocalTty,
        InstantMillis::from_millis(wall()),
    )
    .unwrap();
    let refusal = restore_into_new_root(
        &mut untouched,
        request(&restored, &new_root, true, sessions(&fixture)),
        &mut |entry: &TranscriptEntry| {
            if entry.step == RecoveryStep::PhysicalRebindIntent {
                return Err(());
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(refusal, RestoreRefusal::Transcript);
    assert!(!restore_started(&untouched.transcript, "drill-1"));
    assert_eq!(
        steps(&untouched),
        [
            RecoveryStep::KitOpened,
            RecoveryStep::PreRestoreChecksCompleted
        ]
    );
    assert_eq!(fs::read_dir(&new_root).unwrap().count(), 0);

    // 命令行的写法：已有的先整份写出，之后每追加一条写一行。Auth 撤权的「开始」那条交不出去（比如标准输出
    // 断了）：那一步不做，人手里的文本停在它前面，内存里那条也撤掉。
    let mut text = text_of(&held.transcript);
    let refusal = {
        let mut deliver = written(&mut text);
        restore_into_new_root(
            &mut held,
            request(&restored, &new_root, true, sessions(&fixture)),
            &mut |entry: &TranscriptEntry| {
                if entry.step == RecoveryStep::OwnerRestoreIntent {
                    return Err(());
                }
                deliver(entry)
            },
        )
        .unwrap_err()
    };
    assert_eq!(refusal, RestoreRefusal::Transcript);
    assert_eq!(
        steps(&held).last(),
        Some(&RecoveryStep::PhysicalRebindOutcome)
    );
    assert!(restore_started(&held.transcript, "drill-1"));
    assert_eq!(gate(&fixture, &new_root), Startup::AuthMissing);
    let mut resumed = read_back(&text);
    assert_eq!(resumed.transcript, held.transcript);

    // 接手：拿人手里那份读回来，照常写一份新的完整文本。Auth 没开始过，按清单水位核过再撤权，版本恰好加一。
    let mut lines = text_of(&resumed.transcript);
    let mut resume = request(&restored, &new_root, true, sessions(&fixture));
    resume.resume_interrupted = true;
    let report = restore_into_new_root(&mut resumed, resume, &mut written(&mut lines)).unwrap();
    assert_eq!(report.invalidation.epoch, old_epoch + 1);
    assert_eq!(gate(&fixture, &new_root), Startup::Serves);
    assert_eq!(read_back(&lines).transcript, resumed.transcript);

    // 最终切换：还没开始就完成、拿别的恢复标识，都是次序不对；开始只记一次。
    let mut held = read_back(&lines);
    assert_eq!(
        finish_cutover(
            &mut held,
            cutover(&fixture, &new_root),
            &mut written(&mut lines)
        )
        .unwrap_err(),
        CutoverRefusal::Sequence
    );
    let foreign = Cutover {
        restore_id: "drill-2",
        ..cutover(&fixture, &new_root)
    };
    assert_eq!(
        begin_cutover(&mut held, foreign, &mut written(&mut lines)).unwrap_err(),
        CutoverRefusal::Sequence
    );
    begin_cutover(
        &mut held,
        cutover(&fixture, &new_root),
        &mut written(&mut lines),
    )
    .unwrap();
    assert_eq!(steps(&held).last(), Some(&RecoveryStep::FinalCutoverIntent));
    assert_eq!(
        begin_cutover(
            &mut held,
            cutover(&fixture, &new_root),
            &mut written(&mut lines)
        )
        .unwrap_err(),
        CutoverRefusal::Sequence
    );
    // 主人还没在新根上登录过：完成拒绝，记录不多一行。
    let before = held.transcript.entries.len();
    assert_eq!(
        finish_cutover(
            &mut held,
            cutover(&fixture, &new_root),
            &mut written(&mut lines)
        )
        .unwrap_err(),
        CutoverRefusal::NotReauthenticated
    );
    assert_eq!(held.transcript.entries.len(), before);
    // 登录一次之后完成：签发代次就是撤权之后的身份版本，只有人侧三件，没有身份库和策略库要核。
    login(&fixture, &new_root).await;
    let facts = finish_cutover(
        &mut held,
        cutover(&fixture, &new_root),
        &mut written(&mut lines),
    )
    .unwrap();
    assert!(facts.reauthenticated && facts.old_sessions_invalidated && facts.l3_invalidated);
    assert_eq!(
        facts.issuance_generation,
        u64::try_from(report.invalidation.epoch).unwrap()
    );
    assert!(!facts.agent_store_included && !facts.policy_included);
    assert_eq!(
        steps(&held).last(),
        Some(&RecoveryStep::FinalCutoverOutcome)
    );
    assert_eq!(
        finish_cutover(
            &mut held,
            cutover(&fixture, &new_root),
            &mut written(&mut lines)
        )
        .unwrap_err(),
        CutoverRefusal::Sequence
    );
    // 人手里的文本和内存里那份一模一样，链自洽。
    assert_eq!(read_back(&lines).transcript, held.transcript);
}

/// Auth 撤权的两个断点（Codex 复看修复时指出没有用例）：改完正式名、完成那条交不出去时停下，和撤权做完、还没
/// 改名就停下（用改回暂用名模拟）。前一种撤权已经生效，启动闸门放行，这是「失败就起不来」唯一的例外，但最终
/// 切换要那一条、开始不了；后一种按正式路径起不来。两种都拿人手里的记录接手做完：撤权幂等、身份版本不再加，
/// 完成那条只记一次。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_resumes_after_its_invalidation_and_after_its_rename() {
    let (fixture, _human, sealed, old_epoch) = human_only().await;
    let fetched = target(&fixture, "fetched").join("copy");
    fetch(&sealed.ticket().transfer_source.root, &fetched);
    let restored = RestoredCheckpoint::open(&fetched, sealed.reference(), deadline()).unwrap();
    for renamed in [true, false] {
        let new_root = target(&fixture, if renamed { "renamed" } else { "not-renamed" });
        let mut held = grant(sealed.reference());
        let mut text = text_of(&held.transcript);
        let refusal = {
            let mut deliver = written(&mut text);
            restore_into_new_root(
                &mut held,
                request(&restored, &new_root, true, sessions(&fixture)),
                &mut |entry: &TranscriptEntry| {
                    if entry.step == RecoveryStep::AuthInvalidated {
                        return Err(());
                    }
                    deliver(entry)
                },
            )
            .unwrap_err()
        };
        assert_eq!(refusal, RestoreRefusal::Transcript);
        assert_eq!(steps(&held).last(), Some(&RecoveryStep::OwnerRestoreIntent));
        assert_eq!(names(&new_root), ["audit", "auth", "data", "vault"]);
        if renamed {
            assert_eq!(gate(&fixture, &new_root), Startup::Serves);
            assert_eq!(
                begin_cutover(&mut held, cutover(&fixture, &new_root), &mut |_| Ok(()))
                    .unwrap_err(),
                CutoverRefusal::Sequence
            );
        } else {
            fs::rename(new_root.join("auth"), new_root.join("auth.held")).unwrap();
            assert_eq!(gate(&fixture, &new_root), Startup::AuthMissing);
        }
        let mut resumed = read_back(&text);
        let mut resume = request(&restored, &new_root, true, sessions(&fixture));
        resume.resume_interrupted = true;
        let report = restore(&mut resumed, resume).unwrap();
        assert_eq!(report.invalidation.epoch, old_epoch + 1);
        assert_eq!(names(&new_root), ["audit", "auth", "data", "vault"]);
        assert_eq!(gate(&fixture, &new_root), Startup::Serves);
        let completed = steps(&resumed)
            .into_iter()
            .filter(|step| *step == RecoveryStep::AuthInvalidated)
            .count();
        assert_eq!(completed, 1);
        begin_cutover(&mut resumed, cutover(&fixture, &new_root), &mut |_| Ok(())).unwrap();
    }
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
    let report = restore(
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
    // Auth 和身份库都在失效做完之后改成了正式名，暂用名不再留着。
    assert_eq!(
        names(&new_root),
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
    assert_eq!(gate(fixture, &new_root), Startup::Serves);
    // 最终切换按恢复记录里的名单核：身份库、策略库都在名单里，它们的失效都记过才完成。
    begin_cutover(&mut held, cutover(fixture, &new_root), &mut |_| Ok(())).unwrap();
    login(fixture, &new_root).await;
    let facts = finish_cutover(&mut held, cutover(fixture, &new_root), &mut |_| Ok(())).unwrap();
    assert!(facts.agent_store_included && facts.agent_type_tokens_invalidated);
    assert!(facts.agent_instances_invalidated);
    assert!(facts.policy_included && facts.policy_revocations_reconciled);
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 被封存的根本身是恢复出来的（恢复标识 drill-0；Auth 里另有 drill-9 的旧记录）。以前它的 lineage 被当成这次
/// 恢复的、整次拒绝（Codex 第二意见第 5 条）；现在只当上一环。用过的恢复标识再用：在 lineage 那一串里的由 Broker
/// 在重绑定之前拒绝，只在 Auth 里的由 Auth 撤权开始之前拒绝；两种都停在 Auth 还在暂用名下（第 1、2 条）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restoring_a_restored_roots_checkpoint_links_the_lineage_and_refuses_reused_ids() {
    let (fixture, _human, sealed, old_epoch) = seal_human(true).await;
    let fetched = target(&fixture, "fetched").join("copy");
    fetch(&sealed.ticket().transfer_source.root, &fetched);
    let restored = RestoredCheckpoint::open(&fetched, sealed.reference(), deadline()).unwrap();

    // lineage 那一串里有的 drill-0：Broker 在写 lineage 之前拒绝。data 里还是被封存的根自己的 lineage，新根
    // 没重绑定、Broker 打不开，Auth 在暂用名下，按正式路径起不来。
    let first = target(&fixture, "reuse-lineage");
    let mut held = grant(sealed.reference());
    assert_eq!(
        restore(
            &mut held,
            request_as(&restored, &first, sessions(&fixture), "drill-0")
        )
        .unwrap_err(),
        RestoreRefusal::RestoreIdUsed
    );
    assert_eq!(
        steps(&held),
        [
            RecoveryStep::KitOpened,
            RecoveryStep::PreRestoreChecksCompleted,
            RecoveryStep::PhysicalRebindIntent,
        ]
    );
    let ancestor = read_restore_lineage(&first.join("data")).unwrap().unwrap();
    assert_eq!(ancestor.restore_id.as_deref(), Some("drill-0"));
    assert_eq!(ancestor.checkpoint_id, "an-older-checkpoint");
    assert_eq!(names(&first), ["audit", "auth.held", "data", "vault"]);
    assert!(FsVaultBroker::open_existing(&first.join("vault"), &first.join("data")).is_err());
    assert_eq!(gate(&fixture, &first), Startup::AuthMissing);
    // 接手解决不了：重绑定前重核已放好的组件照样通过（一个字节都没动过），到 Broker 照样按那一串拒绝。
    let mut resume = request_as(&restored, &first, sessions(&fixture), "drill-0");
    resume.resume_interrupted = true;
    assert_eq!(
        restore(&mut held, resume).unwrap_err(),
        RestoreRefusal::RestoreIdUsed
    );
    assert_eq!(
        steps(&held).last(),
        Some(&RecoveryStep::PhysicalRebindIntent)
    );
    assert_eq!(
        read_restore_lineage(&first.join("data"))
            .unwrap()
            .unwrap()
            .restore_id
            .as_deref(),
        Some("drill-0")
    );
    assert_eq!(gate(&fixture, &first), Startup::AuthMissing);

    // 只在 Auth 里有的 drill-9：重绑定做完了，Auth 撤权开始之前按 Auth 自己的记录拒绝，「开始」不记。这时
    // lineage 记的是 drill-9、Auth 里正好有 drill-9 的旧记录：拿暂用名去问，启动闸门会认旧记录放行——按正式路径
    // 打不开 Auth 才是挡住它的那一道。接手也一样拒，不会因为上一次拒过就跳过核对。
    let second = target(&fixture, "reuse-auth");
    let mut held = grant(sealed.reference());
    assert_eq!(
        restore(
            &mut held,
            request_as(&restored, &second, sessions(&fixture), "drill-9")
        )
        .unwrap_err(),
        RestoreRefusal::RestoreIdUsed
    );
    assert_eq!(
        steps(&held).last(),
        Some(&RecoveryStep::PhysicalRebindOutcome)
    );
    assert_eq!(gate(&fixture, &second), Startup::AuthMissing);
    assert_eq!(gate_at(&fixture, &second, "auth.held"), Startup::Serves);
    let mut resume = request_as(&restored, &second, sessions(&fixture), "drill-9");
    resume.resume_interrupted = true;
    assert_eq!(
        restore(&mut held, resume).unwrap_err(),
        RestoreRefusal::RestoreIdUsed
    );
    assert!(!steps(&held).contains(&RecoveryStep::OwnerRestoreIntent));
    assert_eq!(gate(&fixture, &second), Startup::AuthMissing);

    // 没用过的 drill-1：整轮做完，lineage 的上一环是 drill-0、之前用过的那一串记着它；接手一次不再重绑。
    let third = target(&fixture, "second-generation");
    let mut held = grant(sealed.reference());
    let report = restore(
        &mut held,
        request(&restored, &third, true, sessions(&fixture)),
    )
    .unwrap();
    assert_eq!(report.lineage.restore_id.as_deref(), Some("drill-1"));
    assert_eq!(report.lineage.parent_lineage.as_deref(), Some("drill-0"));
    assert_eq!(report.lineage.earlier_restores, ["drill-0"]);
    assert_eq!(
        report.lineage.checkpoint_id,
        sealed.reference().checkpoint_id.as_str()
    );
    assert_eq!(report.invalidation.epoch, old_epoch + 1);
    assert_eq!(names(&third), ["audit", "auth", "data", "vault"]);
    assert_eq!(gate(&fixture, &third), Startup::Serves);
    let mut resume = request(&restored, &third, true, sessions(&fixture));
    resume.resume_interrupted = true;
    let again = restore(&mut held, resume).unwrap();
    assert_eq!(again.lineage, report.lineage);
}

/// 六 owner：放到一半停下的接手（人侧账本放好了、身份库和里面同名的接入方账本还没放）照样接得上——以前按名字
/// 记「放好了哪些」，两个 `audit` 被当成一个，接手时去重核还不存在的接入方账本，永远接不上（Codex 第二意见
/// 第 4 条）。另一个新根：Control 核对过、「开始」交不出去就停了，之后库被人动过：接手时照旧先核、核不过就拒，
/// 再接手一次还是拒，不会把没核过的库当成核过的直接改（第 3 条）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn six_owner_resumes_a_partial_placement_and_never_skips_a_failed_check() {
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

    let partial = target(fixture, "partial");
    let mut held = grant(fact.source());
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
    ] {
        restored
            .extract(component, &partial, name, deadline())
            .unwrap();
    }
    let mut resume = request(&restored, &partial, true, sessions(fixture));
    resume.resume_interrupted = true;
    let report = restore(&mut held, resume).unwrap();
    assert!(report.reused_placement);
    assert!(report.invalidation.audit_confirmed);
    assert_eq!(
        names(&partial),
        [
            "agent", "audit", "auth", "control", "data", "policy", "vault"
        ]
    );
    assert!(partial.join("agent/audit").is_dir());
    assert_eq!(gate(fixture, &partial), Startup::Serves);

    let tampered = target(fixture, "tampered");
    let mut held = grant(fact.source());
    let mut text = text_of(&held.transcript);
    let refusal = {
        let mut deliver = written(&mut text);
        restore_into_new_root(
            &mut held,
            request(&restored, &tampered, true, sessions(fixture)),
            &mut |entry: &TranscriptEntry| {
                if entry.step == RecoveryStep::OwnerRestoreIntent {
                    return Err(());
                }
                deliver(entry)
            },
        )
        .unwrap_err()
    };
    assert_eq!(refusal, RestoreRefusal::Transcript);
    assert_eq!(
        steps(&held).last(),
        Some(&RecoveryStep::PhysicalRebindOutcome)
    );
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
    let expected = restored
        .manifest()
        .watermarks
        .iter()
        .find(|mark| matches!(mark, OwnerWatermark::Control { .. }))
        .unwrap()
        .clone();
    {
        let mut db = ControlDb::open(tampered.join("control/control.sqlite3"), vault_uuid).unwrap();
        db.create_task(
            TaskId::new(),
            &serde_json::json!({"adapter": "consumer"}),
            1,
        )
        .unwrap();
        assert_ne!(db.restored_watermark().ok(), Some(expected));
    }
    let mut resumed = read_back(&text);
    for _ in 0..2 {
        let before = resumed.transcript.entries.len();
        let mut resume = request(&restored, &tampered, true, sessions(fixture));
        resume.resume_interrupted = true;
        assert_eq!(
            restore(&mut resumed, resume).unwrap_err(),
            RestoreRefusal::Owner(Owner::Control)
        );
        assert!(!steps(&resumed)[before..].contains(&RecoveryStep::OwnerRestoreIntent));
    }
    assert_eq!(gate(fixture, &tampered), Startup::AuthMissing);
    assert!(tampered.join("agent.held").is_dir() && !tampered.join("agent").exists());
    stage.assembled.workbench.shutdown().await.unwrap();
}

/// 身份库的两个断点：改完正式名、完成那条交不出去时停下，和吊销做完、还没改名就停下（用改回暂用名模拟，接入方
/// 账本跟着它走）。Auth 还没撤权，两种都按正式路径起不来；都拿人手里的记录接手做完：吊销幂等，完成那条只记一次。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_store_resumes_after_its_invalidation_and_after_its_rename() {
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
    let all = [
        "agent", "audit", "auth", "control", "data", "policy", "vault",
    ];
    for renamed in [true, false] {
        let new_root = target(
            fixture,
            if renamed {
                "agent-renamed"
            } else {
                "agent-not-renamed"
            },
        );
        let mut held = grant(fact.source());
        let mut text = text_of(&held.transcript);
        let refusal = {
            let mut deliver = written(&mut text);
            restore_into_new_root(
                &mut held,
                request(&restored, &new_root, true, sessions(fixture)),
                &mut |entry: &TranscriptEntry| {
                    if entry.step == RecoveryStep::AgentCredentialsInvalidated {
                        return Err(());
                    }
                    deliver(entry)
                },
            )
            .unwrap_err()
        };
        assert_eq!(refusal, RestoreRefusal::Transcript);
        assert_eq!(steps(&held).last(), Some(&RecoveryStep::OwnerRestoreIntent));
        assert!(new_root.join("agent/audit").is_dir() && !new_root.join("agent.held").exists());
        if !renamed {
            fs::rename(new_root.join("agent"), new_root.join("agent.held")).unwrap();
        }
        assert_eq!(gate(fixture, &new_root), Startup::AuthMissing);
        let mut resumed = read_back(&text);
        let mut resume = request(&restored, &new_root, true, sessions(fixture));
        resume.resume_interrupted = true;
        let report = restore(&mut resumed, resume).unwrap();
        assert!(report.invalidation.audit_confirmed);
        assert_eq!(names(&new_root), all);
        assert!(new_root.join("agent/audit").is_dir());
        assert_eq!(gate(fixture, &new_root), Startup::Serves);
        let completed = steps(&resumed)
            .into_iter()
            .filter(|step| *step == RecoveryStep::AgentCredentialsInvalidated)
            .count();
        assert_eq!(completed, 1);
    }
    stage.assembled.workbench.shutdown().await.unwrap();
}
