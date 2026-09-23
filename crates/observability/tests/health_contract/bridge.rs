//! 健康页第六节「双向 Bridge」的行为／失败契约（ADR-025 D11）。

use super::*;

#[test]
fn 驱动平面未装配时第六节未知且整体不显示绿() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let snapshot = studiolo_observability::health::WhitelistSnapshot::default();
    let diagnosis = diagnose(&snapshot).expect("空白名单必须可诊断");
    let mut inputs = healthy_inputs(&owners);
    inputs.whitelist = Some(&diagnosis);
    inputs.bridge = None;

    let report = aggregate(&inputs).expect("固定正文必须可构造");

    assert_eq!(
        section(report.sections(), "sync-bridge").state(),
        HealthState::Unknown
    );
    assert_ne!(report.overall(), HealthState::Normal);
}

#[test]
fn 第六节任一字段缺失判未知而熔断残留未知终态与非干净退出判降级() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let missing = BridgeEvidence {
        mirror_bytes: None,
        ..NORMAL_BRIDGE
    };
    let fused = BridgeEvidence {
        fused: Some(true),
        ..NORMAL_BRIDGE
    };
    let residue = BridgeEvidence {
        residue_guard: Some(ResidueGuard::Blocked),
        ..NORMAL_BRIDGE
    };
    let unknown_jobs = BridgeEvidence {
        unknown_jobs: Some(2),
        ..NORMAL_BRIDGE
    };
    let exit = BridgeEvidence {
        last_exit: Some(ExitClass::NonZero),
        ..NORMAL_BRIDGE
    };
    for (evidence, expected) in [
        (&missing, HealthState::Unknown),
        (&fused, HealthState::Degraded),
        (&residue, HealthState::Degraded),
        (&unknown_jobs, HealthState::Degraded),
        (&exit, HealthState::Degraded),
    ] {
        let mut inputs = healthy_inputs(&owners);
        inputs.bridge = Some(evidence);
        let report = aggregate(&inputs).expect("固定正文必须可构造");
        assert_eq!(
            section(report.sections(), "sync-bridge").state(),
            expected,
            "{evidence:?}"
        );
    }
    let mut inputs = healthy_inputs(&owners);
    inputs.bridge = Some(&residue);
    let report = aggregate(&inputs).expect("固定正文必须可构造");
    assert_eq!(
        section(report.sections(), "sync-bridge").detail(),
        "排除项清除前有镜像残留"
    );
}

#[test]
fn 第六节正常时正文带序号与非账号用量字样() {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let inputs = healthy_inputs(&owners);

    let report = aggregate(&inputs).expect("固定正文必须可构造");
    let bridge = section(report.sections(), "sync-bridge");

    assert_eq!(bridge.state(), HealthState::Normal);
    assert_eq!(bridge.title(), "双向 Bridge");
    assert!(bridge.detail().starts_with("第 3 批于 9990 闭合"));
    assert!(bridge.detail().contains("（非账号用量）"));
    assert!(bridge.detail().ends_with("模式 mirror-remote"));
}
