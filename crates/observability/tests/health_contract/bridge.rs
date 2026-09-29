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

/// 对齐备证据只改一个字段的动作。
type Change = fn(&mut BridgeEvidence);

/// 只改一个字段的证据；其余字段与 `NORMAL_BRIDGE` 相同。
fn variant(change: Change) -> BridgeEvidence {
    let mut evidence = NORMAL_BRIDGE;
    change(&mut evidence);
    evidence
}

/// 用给定证据聚合一次，取第六节的（状态，正文）。
fn judge(evidence: &BridgeEvidence) -> (HealthState, String) {
    let owners = [owner("backup-worker", Some(9_950), 600)];
    let mut inputs = healthy_inputs(&owners);
    inputs.bridge = Some(evidence);
    let report = aggregate(&inputs).expect("固定正文必须可构造");
    let bridge = section(report.sections(), "sync-bridge");
    (bridge.state(), bridge.detail().to_owned())
}

#[test]
fn 第六节十一个字段任一单独缺失都判未知并点名缺的字段() {
    let blank: [(&str, Change); 11] = [
        ("配置核对", |e| e.config_matches = None),
        ("最近一批", |e| e.last_batch = None),
        ("删除熔断", |e| e.fused = None),
        ("终态未知动作数", |e| e.unknown_jobs = None),
        ("静止证据", |e| e.quiescence_valid = None),
        ("冲突副本数", |e| e.conflict_copies = None),
        ("镜像字节数", |e| e.mirror_bytes = None),
        ("退出分类", |e| e.last_exit = None),
        ("恢复协议状态", |e| e.recovery = None),
        ("排除项守卫", |e| e.residue_guard = None),
        ("镜像模式", |e| e.mode = None),
    ];
    for (missing, blank_out) in blank {
        let (state, detail) = judge(&variant(blank_out));
        let expected = format!("Bridge 证据不完整：缺{missing}");
        assert_eq!(state, HealthState::Unknown, "{missing}");
        assert_eq!(detail, expected, "{missing}");
    }
}

#[test]
fn 恢复协议处于强制恢复或退避时第六节降级并写明所处状态() {
    let cases: [(Change, &str); 2] = [
        (
            |e| e.recovery = Some(RecoveryState::ForcedMirrorRemote),
            "强制只下载恢复一次",
        ),
        (|e| e.recovery = Some(RecoveryState::Backoff), "退避等待"),
    ];
    for (change, label) in cases {
        let (state, detail) = judge(&variant(change));
        let expected = format!("恢复协议进行中：{label}");
        assert_eq!(state, HealthState::Degraded, "{label}");
        assert_eq!(detail, expected, "{label}");
    }
}

#[test]
fn 五种非干净退出都降级且正文带对应分类() {
    for exit in [
        ExitClass::NonZero,
        ExitClass::Signal,
        ExitClass::DeadlineExceeded,
        ExitClass::OwnerStopped,
        ExitClass::Unconfirmed,
    ] {
        let mut evidence = NORMAL_BRIDGE;
        evidence.last_exit = Some(exit);
        let (state, detail) = judge(&evidence);
        let expected = format!("上次 headless 运行{}", exit.label());
        assert_eq!(state, HealthState::Degraded, "{exit:?}");
        assert_eq!(detail, expected, "{exit:?}");
    }
}

#[test]
fn owner关停时被停止的运行判降级且正文写明() {
    let (state, detail) = judge(&variant(|e| e.last_exit = Some(ExitClass::OwnerStopped)));

    assert_eq!(state, HealthState::Degraded);
    assert_eq!(detail, "上次 headless 运行在 owner 关停时被停止");
}

#[test]
fn 静止证据过期时第六节判未知而不是判降级或正常() {
    let (state, detail) = judge(&variant(|e| e.quiescence_valid = Some(false)));

    assert_eq!(state, HealthState::Unknown);
    assert_eq!(detail, "静止证据已过期，等待下一轮重新取得");
}

#[test]
fn 多项异常并存时按熔断残留未知终态退出恢复静止的固定顺序取第一项() {
    let mut evidence = BridgeEvidence {
        fused: Some(true),
        residue_guard: Some(ResidueGuard::Blocked),
        config_matches: Some(false),
        unknown_jobs: Some(1),
        last_exit: Some(ExitClass::Signal),
        recovery: Some(RecoveryState::Backoff),
        quiescence_valid: Some(false),
        ..NORMAL_BRIDGE
    };
    let expected = [
        "删除熔断生效，本批与后续批次全部阻断",
        "排除项清除前有镜像残留",
        "headless 配置与审批文档不一致，本轮未同步",
        "有 1 条动作终态未知，等待原键核对",
        "上次 headless 运行被信号终止",
        "恢复协议进行中：退避等待",
        "静止证据已过期，等待下一轮重新取得",
    ];
    for (index, detail) in expected.iter().enumerate() {
        assert_eq!(judge(&evidence).1, *detail, "第 {index} 项");
        match index {
            0 => evidence.fused = Some(false),
            1 => evidence.residue_guard = Some(ResidueGuard::Clear),
            2 => evidence.config_matches = Some(true),
            3 => evidence.unknown_jobs = Some(0),
            4 => evidence.last_exit = Some(ExitClass::Clean),
            5 => evidence.recovery = Some(RecoveryState::Idle),
            _ => evidence.quiescence_valid = Some(true),
        }
    }
    assert_eq!(judge(&evidence).0, HealthState::Normal);
}

#[test]
fn 三组标签取值固定且不含路径邮箱形状或控制字符() {
    let labels = [
        (ExitClass::Clean.label(), "干净退出"),
        (ExitClass::NonZero.label(), "以非零退出码结束"),
        (ExitClass::Signal.label(), "被信号终止"),
        (ExitClass::DeadlineExceeded.label(), "超过时限被停止"),
        (ExitClass::OwnerStopped.label(), "在 owner 关停时被停止"),
        (ExitClass::Unconfirmed.label(), "退出结果未确认"),
        (RecoveryState::Idle.label(), "空闲"),
        (
            RecoveryState::ForcedMirrorRemote.label(),
            "强制只下载恢复一次",
        ),
        (RecoveryState::Backoff.label(), "退避等待"),
        (ModeLabel::MirrorRemote.label(), "mirror-remote"),
        (ModeLabel::Bidirectional.label(), "bidirectional"),
    ];
    for (actual, expected) in labels {
        assert_eq!(actual, expected);
        assert!(HealthSection::new("x", "测试", HealthState::Normal, actual).is_ok());
    }
}

#[test]
fn 双向模式下正常正文以对应模式标签结尾() {
    let (state, detail) = judge(&variant(|e| e.mode = Some(ModeLabel::Bidirectional)));

    assert_eq!(state, HealthState::Normal);
    assert!(detail.ends_with("模式 bidirectional"));
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
