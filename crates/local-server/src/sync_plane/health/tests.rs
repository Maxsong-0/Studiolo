//! `PlaneHealth` 的回填契约：每个方法只写它能证明的字段，轮末收口不把失败轮洗成正常。

use super::{BridgeHealthProbe, OWNER_LABEL, PlaneHealth};
use crate::sync_plane::PlaneError;
use crate::sync_plane::driver::RoundOutcome;
use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_observability::health::{
    BridgeEvidence, ExitClass, HealthState, Instant, ModeLabel, OwnerLiveness, RecoveryState,
    ResidueGuard, bridge_section,
};
use studiolo_observability::quota::SyncUsage;
use studiolo_sync_transport::owned_run::SyncMode;

const QUOTA: ManualQuota = ManualQuota {
    account_limit_bytes: 10_000,
    used_bytes_manual: 1_000,
    recorded_at_unix: 1_700_000_000,
};

fn usage() -> SyncUsage {
    SyncUsage {
        objects: 12,
        bytes: 4_096,
    }
}

fn settled(sequence: u64) -> Result<RoundOutcome, PlaneError> {
    Ok(RoundOutcome::Settled {
        sequence,
        baseline_advanced: true,
    })
}

fn unknown(sequence: u64) -> Result<RoundOutcome, PlaneError> {
    Ok(RoundOutcome::Unknown { sequence })
}

fn blocked(sequence: u64) -> Result<RoundOutcome, PlaneError> {
    Ok(RoundOutcome::Blocked { sequence })
}

/// 一整轮干净走完之前驱动器会做的四次回填。
fn fully_reported(health: &PlaneHealth, sequence: u64) {
    health.record_mode(SyncMode::MirrorRemote);
    health.record_headless(true, false, false);
    health.record_usage(usage());
    health.record_execution(sequence, 0, 0, false);
}

/// 先干净闭合一批，让所有字段都处于"正常"，再看后续事件能不能把绿压下去。
fn after_clean_batch() -> PlaneHealth {
    let health = PlaneHealth::new(None);
    fully_reported(&health, 1);
    health.record_round(10_000, &settled(1));
    health
}

fn state(health: &PlaneHealth) -> HealthState {
    bridge_section(Some(&health.bridge()))
        .expect("固定正文必须可构造")
        .state()
}

/// 某条失败应当命中的字段断言。
type Hit = fn(&BridgeEvidence) -> bool;

#[test]
fn 新建时第六节全部字段为空且活性没有回执() {
    let health = PlaneHealth::new(Some(QUOTA));

    assert_eq!(health.bridge(), BridgeEvidence::default());
    assert_eq!(health.liveness().last_receipt, None);
    // 写者边界词法门禁禁止本目录用点号访问 owner 字段，改用解构比对角色标签。
    let OwnerLiveness { owner: label, .. } = health.liveness();
    assert_eq!(label.as_str(), OWNER_LABEL);
    assert_eq!(health.liveness().expected_period_seconds, 600);
    assert_eq!(health.sync_usage(), None);
    assert_eq!(health.manual_quota(), Some(QUOTA));
    assert_eq!(state(&health), HealthState::Unknown);
}

#[test]
fn 一批闭合且轮内回填齐全后第六节正常并盖上序号与时刻() {
    let health = PlaneHealth::new(None);
    fully_reported(&health, 7);

    health.record_round(10_000, &settled(7));

    let evidence = health.bridge();
    let batch = evidence.last_batch.map(|b| (b.sequence, b.unix_seconds));
    assert_eq!(batch, Some((7, 10_000)));
    assert_eq!(evidence.quiescence_valid, Some(true));
    assert_eq!(evidence.mode, Some(ModeLabel::MirrorRemote));
    assert_eq!(evidence.mirror_bytes, Some(4_096));
    assert_eq!(state(&health), HealthState::Normal);
    let receipt = health.liveness().last_receipt;
    assert_eq!(receipt, Some(Instant::from_unix_seconds(10_000)));
    assert_eq!(health.sync_usage(), Some(usage()));
}

#[test]
fn 每种失败只落到对应字段且任何失败轮都不留静止票据() {
    let cases: [(PlaneError, Hit); 5] = [
        (PlaneError::Fused, |e| e.fused == Some(true)),
        (PlaneError::ConfigMismatch, |e| {
            e.config_matches == Some(false)
        }),
        (PlaneError::ResidueBlocked, |e| {
            e.residue_guard == Some(ResidueGuard::Blocked)
        }),
        (PlaneError::Backoff, |e| {
            e.recovery == Some(RecoveryState::Backoff)
        }),
        (PlaneError::Transport, |e| {
            e.last_exit == Some(ExitClass::Unconfirmed)
        }),
    ];
    for (error, hit) in cases {
        let health = after_clean_batch();

        health.record_round(10_300, &Err(error));

        let evidence = health.bridge();
        assert!(hit(&evidence), "{error:?}");
        assert_eq!(evidence.quiescence_valid, Some(false), "{error:?}");
        assert_eq!(state(&health), HealthState::Degraded, "{error:?}");
    }
}

#[test]
fn 没有专属字段的失败也靠静止票据落到未知而不是保持正常() {
    let errors = [
        PlaneError::Configuration,
        PlaneError::Quiescence,
        PlaneError::Observation,
        PlaneError::Audit,
        PlaneError::Storage,
        PlaneError::Unavailable,
    ];
    for error in errors {
        let health = after_clean_batch();

        health.record_round(10_300, &Err(error));

        assert_eq!(state(&health), HealthState::Unknown, "{error:?}");
    }
}

#[test]
fn 退避跳过的轮次记为退避等待() {
    let health = after_clean_batch();

    health.record_round(10_300, &Ok(RoundOutcome::Skipped));

    assert_eq!(health.bridge().recovery, Some(RecoveryState::Backoff));
    assert_eq!(state(&health), HealthState::Degraded);
}

#[test]
fn 终态未知的批次至少记一条且执行段报了数就以报的为准() {
    let health = after_clean_batch();

    health.record_round(10_300, &unknown(2));
    assert_eq!(health.bridge().unknown_jobs, Some(1));

    health.record_execution(3, 4, 0, false);
    health.record_round(10_600, &unknown(3));
    assert_eq!(health.bridge().unknown_jobs, Some(4));

    health.record_round(10_900, &Err(PlaneError::Unknown));
    assert_eq!(health.bridge().unknown_jobs, Some(4));
    assert_eq!(state(&health), HealthState::Degraded);
}

#[test]
fn 冻结的批次若执行段没报本批就不敢说熔断未生效() {
    let health = after_clean_batch();
    assert_eq!(health.bridge().fused, Some(false));

    health.record_round(10_300, &blocked(2));

    assert_eq!(health.bridge().fused, None);
    assert_eq!(state(&health), HealthState::Unknown);

    health.record_execution(3, 0, 2, true);
    health.record_round(10_600, &blocked(3));

    assert_eq!(health.bridge().fused, Some(true));
    assert_eq!(health.bridge().conflict_copies, Some(2));
    assert_eq!(state(&health), HealthState::Degraded);
}

#[test]
fn 空转轮次只证明守卫退出与协议状态不碰熔断与终态计数() {
    let health = PlaneHealth::new(None);
    health.record_mode(SyncMode::Bidirectional);
    health.record_usage(usage());

    health.record_round(10_000, &Ok(RoundOutcome::Idle));

    let evidence = health.bridge();
    assert_eq!(evidence.residue_guard, Some(ResidueGuard::Clear));
    assert_eq!(evidence.last_exit, Some(ExitClass::Clean));
    assert_eq!(evidence.recovery, Some(RecoveryState::Idle));
    assert_eq!(evidence.quiescence_valid, Some(true));
    assert_eq!(evidence.last_batch, None);
    assert_eq!(evidence.fused, None);
    assert_eq!(evidence.unknown_jobs, None);
    assert_eq!(state(&health), HealthState::Unknown);
}

#[test]
fn 熔断与残留在下一批干净闭合前一直压着第六节() {
    let health = after_clean_batch();
    health.record_round(10_300, &Err(PlaneError::Fused));
    health.record_round(10_600, &Err(PlaneError::Storage));
    assert_eq!(health.bridge().fused, Some(true));

    fully_reported(&health, 2);
    health.record_round(10_900, &settled(2));

    assert_eq!(health.bridge().fused, Some(false));
    assert_eq!(state(&health), HealthState::Normal);
}

#[test]
fn headless回收结果按三个布尔保守分类且强制恢复运行记为协议进行中() {
    let health = PlaneHealth::new(None);
    let cases = [
        ((true, false, false), ExitClass::Clean, RecoveryState::Idle),
        (
            (false, true, false),
            ExitClass::NonZero,
            RecoveryState::Idle,
        ),
        (
            (false, false, false),
            ExitClass::Unconfirmed,
            RecoveryState::Idle,
        ),
        (
            (true, false, true),
            ExitClass::Clean,
            RecoveryState::ForcedMirrorRemote,
        ),
    ];
    for ((clean, rc_one, forced), exit, recovery) in cases {
        health.record_headless(clean, rc_one, forced);

        let evidence = health.bridge();
        assert_eq!(evidence.last_exit, Some(exit), "{clean} {rc_one} {forced}");
        assert_eq!(
            evidence.recovery,
            Some(recovery),
            "{clean} {rc_one} {forced}"
        );
    }
}

#[test]
fn 模式回填与传输模式枚举一一对应() {
    let health = PlaneHealth::new(None);

    health.record_mode(SyncMode::MirrorRemote);
    assert_eq!(health.bridge().mode, Some(ModeLabel::MirrorRemote));

    health.record_mode(SyncMode::Bidirectional);
    assert_eq!(health.bridge().mode, Some(ModeLabel::Bidirectional));
}
