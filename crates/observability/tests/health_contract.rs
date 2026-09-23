//! 健康页聚合的行为／失败／恢复三类契约。

use studiolo_observability::health::{
    BatchStamp, BridgeEvidence, ExitClass, HealthInputs, HealthSection, HealthState, Instant,
    ModeLabel, NO_RECOVERY_POINT, OwnerLabel, OwnerLiveness, RecoveryPointEvidence, RecoveryState,
    ResidueGuard, aggregate, diagnose,
};

/// 全字段齐备且无异常的 Bridge 证据；`&NORMAL_BRIDGE` 可直接放进 `HealthInputs`。
const NORMAL_BRIDGE: BridgeEvidence = BridgeEvidence {
    last_batch: Some(BatchStamp {
        sequence: 3,
        unix_seconds: 9_990,
    }),
    fused: Some(false),
    unknown_jobs: Some(0),
    quiescence_valid: Some(true),
    conflict_copies: Some(0),
    mirror_bytes: Some(4_096),
    last_exit: Some(ExitClass::Clean),
    recovery: Some(RecoveryState::Idle),
    residue_guard: Some(ResidueGuard::Clear),
    mode: Some(ModeLabel::MirrorRemote),
};
use studiolo_observability::quota::QuotaVerdict;

fn owner(name: &str, receipt: Option<i64>, period: u64) -> OwnerLiveness {
    OwnerLiveness {
        owner: OwnerLabel::parse(name).expect("固定测试标签必须合法"),
        last_receipt: receipt.map(Instant::from_unix_seconds),
        expected_period_seconds: period,
    }
}

fn healthy_inputs<'a>(owners: &'a [OwnerLiveness]) -> HealthInputs<'a> {
    HealthInputs {
        now: Instant::from_unix_seconds(10_000),
        recovery_point: Some(RecoveryPointEvidence::Established {
            content_cutoff: Instant::from_unix_seconds(9_900),
        }),
        recovery_point_budget_seconds: 3_600,
        owners,
        disk: QuotaVerdict::Normal,
        sync: QuotaVerdict::Normal,
        whitelist: None,
        bridge: Some(&NORMAL_BRIDGE),
    }
}

fn section<'a>(sections: &'a [HealthSection], code: &str) -> &'a HealthSection {
    sections
        .iter()
        .find(|section| section.code() == code)
        .expect("聚合结果必须包含该节")
}

#[test]
fn 证据齐备且全部在预算内时健康页整体显示正常() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let snapshot = studiolo_observability::health::WhitelistSnapshot::default();
    let diagnosis = diagnose(&snapshot).expect("空白名单必须可诊断");
    let mut inputs = healthy_inputs(&owners);
    inputs.whitelist = Some(&diagnosis);

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(report.overall(), HealthState::Normal);
    assert_eq!(report.sections().len(), 6);
}

#[test]
fn 尚未建立恢复点时该节固定显示既定中文并且整体不显示绿() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let mut inputs = healthy_inputs(&owners);
    inputs.recovery_point = Some(RecoveryPointEvidence::NotEstablished);

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    let recovery = section(report.sections(), "recovery-point");
    assert_eq!(recovery.detail(), NO_RECOVERY_POINT);
    assert_eq!(recovery.state(), HealthState::Unknown);
    assert_ne!(report.overall(), HealthState::Normal);
}

#[test]
fn 恢复点证据完全缺失时该节判未知而不是判正常() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let mut inputs = healthy_inputs(&owners);
    inputs.recovery_point = None;

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "recovery-point").state(),
        HealthState::Unknown
    );
    assert_eq!(report.overall(), HealthState::Unknown);
}

#[test]
fn 本地时钟早于恢复点内容截止时刻时判定为时间证据不可信() {
    let owners = [owner("backup-worker", Some(8_000), 600)];
    let mut inputs = healthy_inputs(&owners);
    inputs.now = Instant::from_unix_seconds(8_500);
    inputs.recovery_point = Some(RecoveryPointEvidence::Established {
        content_cutoff: Instant::from_unix_seconds(9_900),
    });

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    let recovery = section(report.sections(), "recovery-point");
    assert_eq!(recovery.state(), HealthState::Unknown);
    assert!(recovery.detail().contains("时间证据不可信"));
}

#[test]
fn 恢复点年龄超过预算时该节降级而不是保持正常() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let mut inputs = healthy_inputs(&owners);
    inputs.recovery_point_budget_seconds = 10;

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "recovery-point").state(),
        HealthState::Degraded
    );
    assert_eq!(report.overall(), HealthState::Degraded);
}

#[test]
fn 没有任何owner活性证据时活性节判未知() {
    let inputs = healthy_inputs(&[]);

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "owner-liveness").state(),
        HealthState::Unknown
    );
}

#[test]
fn 存在超期owner时活性节降级且压过证据不足的未知() {
    let owners = [
        owner("backup-worker", Some(1_000), 600),
        owner("sync-transport", None, 600),
    ];
    let inputs = healthy_inputs(&owners);

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "owner-liveness").state(),
        HealthState::Degraded
    );
}

#[test]
fn 配额读数缺失时对应节判未知而超限时判降级() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let mut inputs = healthy_inputs(&owners);
    inputs.disk = QuotaVerdict::Unknown;
    inputs.sync = QuotaVerdict::Exceeded;

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "disk-quota").state(),
        HealthState::Unknown
    );
    assert_eq!(
        section(report.sections(), "sync-quota").state(),
        HealthState::Degraded
    );
}

#[test]
fn 白名单诊断没跑成时该节判未知而不是判一致() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let inputs = healthy_inputs(&owners);

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "whitelist-diagnosis").state(),
        HealthState::Unknown
    );
}

#[test]
fn 公开摘要只含机器码与状态因而装不下路径或主人身份() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let report = aggregate(&healthy_inputs(&owners)).expect("固定正文必须可构造");

    let summary = report.public_summary();

    assert_eq!(summary.sections().len(), report.sections().len());
    for (code, _) in summary.sections() {
        assert!(code.is_ascii());
        assert!(!code.contains('/'));
    }
}

#[test]
fn 结论正文含路径或邮箱形状时构造直接失败() {
    assert!(HealthSection::new("x", "测试", HealthState::Normal, "/private/vault/notes").is_err());
    assert!(HealthSection::new("x", "测试", HealthState::Normal, "owner@example").is_err());
    assert!(HealthSection::new("x", "测试", HealthState::Normal, "").is_err());
}

#[test]
fn 证据从缺失恢复成齐备之后健康页可以重新显示正常() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let snapshot = studiolo_observability::health::WhitelistSnapshot::default();
    let diagnosis = diagnose(&snapshot).expect("空白名单必须可诊断");

    let mut broken = healthy_inputs(&owners);
    broken.recovery_point = None;
    let degraded = aggregate(&broken).expect("固定正文必须可构造");
    assert_ne!(degraded.overall(), HealthState::Normal);

    let mut repaired = healthy_inputs(&owners);
    repaired.whitelist = Some(&diagnosis);
    let recovered = aggregate(&repaired).expect("固定正文必须可构造");

    assert_eq!(recovered.overall(), HealthState::Normal);
}

/// 第六节「双向 Bridge」的契约，独立成文件只为压住单文件行数上限。
/// 集成测试根文件的子模块默认在同级目录找，这里显式指到子目录，免得 cargo 把它当独立测试目标。
#[path = "health_contract/bridge.rs"]
mod bridge;
