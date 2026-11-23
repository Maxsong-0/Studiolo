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

/// 字段齐、多条正证据并存：第一条写整句，其余按同一顺序列在「另有：」之后；逐条解除，顺序不变。
#[test]
fn 字段齐时多项异常并存按固定顺序报第一条并列出其余() {
    let mut evidence = BridgeEvidence {
        deletes_blocked: Some(2),
        fused: Some(true),
        residue_guard: Some(ResidueGuard::Blocked),
        config_matches: Some(false),
        unknown_jobs: Some(1),
        last_exit: Some(ExitClass::Signal),
        recovery: Some(RecoveryState::Backoff),
        quiescence_valid: Some(false),
        ..NORMAL_BRIDGE
    };
    // 熔断生效就排第一（T-222：它持久、开了文件操作也照样挡；字段齐不齐都一样）；熔断解除后删除被挡排第一。
    // 静止证据过期只是「未知」级别，不进「另有」，正证据全部解除后才轮到它。
    let expected = [
        (
            HealthState::Degraded,
            "删除熔断生效，本批与后续批次全部阻断；另有：删除被挡、排除项残留、配置不符、终态未知动作、非干净退出、恢复协议进行中",
        ),
        (
            HealthState::Degraded,
            "远端删了 2 个文件（改名也算），主库没开文件操作，同步整批暂停：到远端恢复这些文件或开文件操作后接着跑；另有：排除项残留、配置不符、终态未知动作、非干净退出、恢复协议进行中",
        ),
        (
            HealthState::Degraded,
            "排除项清除前有镜像残留；另有：配置不符、终态未知动作、非干净退出、恢复协议进行中",
        ),
        (
            HealthState::Degraded,
            "headless 配置与审批文档不一致，本轮未同步；另有：终态未知动作、非干净退出、恢复协议进行中",
        ),
        (
            HealthState::Degraded,
            "有 1 条动作终态未知，等待原键核对；另有：非干净退出、恢复协议进行中",
        ),
        (
            HealthState::Degraded,
            "上次 headless 运行被信号终止；另有：恢复协议进行中",
        ),
        (HealthState::Degraded, "恢复协议进行中：退避等待"),
        (HealthState::Unknown, "静止证据已过期，等待下一轮重新取得"),
    ];
    for (index, (state, detail)) in expected.iter().enumerate() {
        assert_eq!(
            judge(&evidence),
            (*state, (*detail).to_owned()),
            "第 {index} 项"
        );
        match index {
            0 => evidence.fused = Some(false),
            1 => evidence.deletes_blocked = Some(0),
            2 => evidence.residue_guard = Some(ResidueGuard::Clear),
            3 => evidence.config_matches = Some(true),
            4 => evidence.unknown_jobs = Some(0),
            5 => evidence.last_exit = Some(ExitClass::Clean),
            6 => evidence.recovery = Some(RecoveryState::Idle),
            _ => evidence.quiescence_valid = Some(true),
        }
    }
    assert_eq!(judge(&evidence).0, HealthState::Normal);
}

/// BUG-046：被挡的那几轮不走执行段，重启后「最近一批」等字段一直是空的；
/// 删除被挡有正证据就要报出来，不能被「证据不完整」盖住。
#[test]
fn 删除被挡有正证据就判降级不等别的字段齐() {
    let after_restart = BridgeEvidence {
        deletes_blocked: Some(1),
        ..BridgeEvidence::default()
    };
    let (state, detail) = judge(&after_restart);

    assert_eq!(state, HealthState::Degraded);
    assert_eq!(
        detail,
        "远端删了 1 个文件（改名也算），主库没开文件操作，同步整批暂停：到远端恢复这些文件或开文件操作后接着跑；证据不完整：缺最近一批"
    );
    // 复查第 2 轮：重启后熔断冻结着旧批次、续做那一轮守卫仍挡。T-223 起熔断不齐也报得出来，
    // 「字段齐才让位」的条件用户定去掉：熔断照样排第一，被挡跟在「另有」里，两个原因都看得见。
    let fused_after_restart = BridgeEvidence {
        deletes_blocked: Some(2),
        fused: Some(true),
        ..BridgeEvidence::default()
    };
    let (state, detail) = judge(&fused_after_restart);

    assert_eq!(state, HealthState::Degraded);
    assert_eq!(
        detail,
        "删除熔断生效，本批与后续批次全部阻断；另有：删除被挡；证据不完整：缺最近一批"
    );
}

/// T-223：重启后「最近一批」等字段要等执行段闭合一次才补得齐；只要有一条正证据，就报那一条而不是「证据不完整」。
#[test]
fn 重启后只有一条正证据别的字段都空时报那条正证据() {
    let cases: [(Change, &str); 7] = [
        (
            |e| e.deletes_blocked = Some(4),
            "远端删了 4 个文件（改名也算），主库没开文件操作，同步整批暂停：到远端恢复这些文件或开文件操作后接着跑",
        ),
        (
            |e| e.fused = Some(true),
            "删除熔断生效，本批与后续批次全部阻断",
        ),
        (
            |e| e.residue_guard = Some(ResidueGuard::Blocked),
            "排除项清除前有镜像残留",
        ),
        (
            |e| e.config_matches = Some(false),
            "headless 配置与审批文档不一致，本轮未同步",
        ),
        (
            |e| e.unknown_jobs = Some(1),
            "有 1 条动作终态未知，等待原键核对",
        ),
        (
            |e| e.last_exit = Some(ExitClass::Unconfirmed),
            "上次 headless 运行退出结果未确认",
        ),
        (
            |e| e.recovery = Some(RecoveryState::ForcedMirrorRemote),
            "恢复协议进行中：强制只下载恢复一次",
        ),
    ];
    for (only, sentence) in cases {
        let mut evidence = BridgeEvidence::default();
        only(&mut evidence);

        let (state, detail) = judge(&evidence);

        assert_eq!(state, HealthState::Degraded, "{sentence}");
        assert_eq!(detail, format!("{sentence}；证据不完整：缺最近一批"));
    }
}

/// 另一侧：重启后已知的值都不是问题（未熔断、0 条未知、守卫放行、配置一致、干净退出、协议空闲、放行 0 个）时，
/// 仍是「证据不完整」的未知，不冒报降级。
#[test]
fn 重启后已知的值都不是问题时仍判未知不冒报降级() {
    let cases: [Change; 7] = [
        |e| e.deletes_blocked = Some(0),
        |e| e.fused = Some(false),
        |e| e.residue_guard = Some(ResidueGuard::Clear),
        |e| e.config_matches = Some(true),
        |e| e.unknown_jobs = Some(0),
        |e| e.last_exit = Some(ExitClass::Clean),
        |e| e.recovery = Some(RecoveryState::Idle),
    ];
    for only in cases {
        let mut evidence = BridgeEvidence::default();
        only(&mut evidence);

        let (state, detail) = judge(&evidence);

        assert_eq!(state, HealthState::Unknown, "{evidence:?}");
        assert_eq!(detail, "Bridge 证据不完整：缺最近一批", "{evidence:?}");
    }
}

/// 重启后最常见的一种：上次 headless 在 owner 关停时被停下，恢复协议要强制只下载一次，别的字段都还空着。
#[test]
fn 重启后多条正证据并存且字段不齐时按固定顺序报出并点名缺的字段() {
    let evidence = BridgeEvidence {
        last_exit: Some(ExitClass::OwnerStopped),
        recovery: Some(RecoveryState::ForcedMirrorRemote),
        unknown_jobs: Some(3),
        ..BridgeEvidence::default()
    };

    let (state, detail) = judge(&evidence);

    assert_eq!(state, HealthState::Degraded);
    assert_eq!(
        detail,
        "有 3 条动作终态未知，等待原键核对；另有：非干净退出、恢复协议进行中；证据不完整：缺最近一批"
    );
}

/// 正文上限 200 字（`HealthSection::new` 越界即整页聚合失败）：正证据全部并存、计数取 u64 上限、还缺字段，也要在预算内。
/// 最长的是删除被挡排第一（熔断未生效）的那种；七条都在时熔断排第一，被挡退进「另有」。
#[test]
fn 正证据全部并存且缺字段时正文仍在长度预算内() {
    let longest = BridgeEvidence {
        deletes_blocked: Some(u64::MAX),
        residue_guard: Some(ResidueGuard::Blocked),
        config_matches: Some(false),
        unknown_jobs: Some(u64::MAX),
        last_exit: Some(ExitClass::OwnerStopped),
        recovery: Some(RecoveryState::ForcedMirrorRemote),
        ..BridgeEvidence::default()
    };
    let all_seven = BridgeEvidence {
        fused: Some(true),
        ..longest
    };
    let cases = [
        (
            longest,
            "远端删了 18446744073709551615 个文件（改名也算），主库没开文件操作，同步整批暂停：到远端恢复这些文件或开文件操作后接着跑；另有：排除项残留、配置不符、终态未知动作、非干净退出、恢复协议进行中；证据不完整：缺最近一批",
        ),
        (
            all_seven,
            "删除熔断生效，本批与后续批次全部阻断；另有：删除被挡、排除项残留、配置不符、终态未知动作、非干净退出、恢复协议进行中；证据不完整：缺最近一批",
        ),
    ];
    for (evidence, expected) in cases {
        let (state, detail) = judge(&evidence);

        assert_eq!(state, HealthState::Degraded);
        assert_eq!(detail, expected);
        assert!(detail.chars().count() <= 200, "{}", detail.chars().count());
    }
}

/// 「还没有一轮证明过」不等于被挡：别的字段齐备时不因它判未知（每轮走到删除守卫都会写成 0 或被挡的个数）。
#[test]
fn 删除被挡没证明过时不单独阻止正常() {
    let unproven = BridgeEvidence {
        deletes_blocked: None,
        ..NORMAL_BRIDGE
    };

    assert_eq!(judge(&unproven).0, HealthState::Normal);
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

/// T-223 起非干净退出是正证据：重启后只知道它时判降级、先报退出分类，再点名缺的字段；干净退出照旧是未知。
#[test]
fn 重启后证据仍不完整时已知的非干净退出判降级且不假报正常() {
    for exit in [
        ExitClass::OwnerStopped,
        ExitClass::NonZero,
        ExitClass::Signal,
        ExitClass::DeadlineExceeded,
        ExitClass::Unconfirmed,
    ] {
        let evidence = BridgeEvidence {
            last_exit: Some(exit),
            ..BridgeEvidence::default()
        };
        let (state, detail) = judge(&evidence);
        assert_eq!(state, HealthState::Degraded);
        assert_eq!(
            detail,
            format!("上次 headless 运行{}；证据不完整：缺最近一批", exit.label())
        );
    }
    let (state, detail) = judge(&BridgeEvidence {
        last_exit: Some(ExitClass::Clean),
        ..BridgeEvidence::default()
    });
    assert_eq!(state, HealthState::Unknown);
    assert_eq!(detail, "Bridge 证据不完整：缺最近一批");
}

/// BUG-047：三方并集超过路径预算时观察整轮失败、同步停住；这条排在所有正证据前面，也不等别的字段齐。
#[test]
fn 路径超上限排在所有正证据前面且不等字段齐() {
    let over = BridgeEvidence {
        path_usage: Some(PathUsage {
            used: 4108,
            budget: 4096,
        }),
        deletes_blocked: Some(2),
        fused: Some(true),
        ..BridgeEvidence::default()
    };

    assert_eq!(
        judge(&over),
        (
            HealthState::Degraded,
            "这一轮要比对 4108 条路径，超过上限 4096 条，同步停住：要出新版提高上限；另有：删除熔断、删除被挡；证据不完整：缺最近一批".to_owned()
        )
    );
    let full = BridgeEvidence {
        path_usage: over.path_usage,
        fused: Some(true),
        ..NORMAL_BRIDGE
    };
    assert_eq!(
        judge(&full).1,
        "这一轮要比对 4108 条路径，超过上限 4096 条，同步停住：要出新版提高上限；另有：删除熔断",
        "字段齐时也排在熔断前面"
    );
    let exact = variant(|e| {
        e.path_usage = Some(PathUsage {
            used: 4096,
            budget: 4096,
        })
    });
    assert_eq!(judge(&exact).0, HealthState::Degraded, "恰好用满算接近上限");
    assert!(
        judge(&exact)
            .1
            .starts_with("路径用量 4096／4096，已到上限的 100%")
    );
}

/// 到预算的八成就降级提前报（BUG-047），排在别的正证据后面；差一条不到八成照常正常。
#[test]
fn 路径用量到八成判降级并排在其余正证据之后() {
    let near = variant(|e| {
        e.path_usage = Some(PathUsage {
            used: 13_108,
            budget: 16_384,
        })
    });
    assert_eq!(
        judge(&near),
        (
            HealthState::Degraded,
            "路径用量 13108／16384，已到上限的 80%：该安排提高上限".to_owned()
        )
    );
    let below = variant(|e| {
        e.path_usage = Some(PathUsage {
            used: 13_107,
            budget: 16_384,
        })
    });
    assert_eq!(judge(&below).0, HealthState::Normal);
    // 恰好八成（预算 16384 时没有整数恰好落在八成上，换个能整除的预算钉住「含等号」）。
    let eight = variant(|e| {
        e.path_usage = Some(PathUsage {
            used: 8,
            budget: 10,
        })
    });
    assert_eq!(
        judge(&eight),
        (
            HealthState::Degraded,
            "路径用量 8／10，已到上限的 80%：该安排提高上限".to_owned()
        )
    );
    let seven = variant(|e| {
        e.path_usage = Some(PathUsage {
            used: 7,
            budget: 10,
        })
    });
    assert_eq!(judge(&seven).0, HealthState::Normal);
    let with_unknown = BridgeEvidence {
        unknown_jobs: Some(1),
        ..near
    };
    assert_eq!(
        judge(&with_unknown).1,
        "有 1 条动作终态未知，等待原键核对；另有：路径接近上限"
    );
}

/// 正常正文带路径用量，放在模式之前；还没算到过（`None`）不算缺字段、不阻止正常（BUG-047）。
#[test]
fn 正常正文带路径用量且没算到过时不阻止正常() {
    let normal = variant(|e| {
        e.path_usage = Some(PathUsage {
            used: 4108,
            budget: 16_384,
        })
    });
    let (state, detail) = judge(&normal);
    assert_eq!(state, HealthState::Normal);
    assert!(
        detail.ends_with("；路径 4108／16384；模式 mirror-remote"),
        "{detail}"
    );
    let unproven = variant(|e| e.path_usage = None);
    assert_eq!(judge(&unproven).0, HealthState::Normal);
    assert!(!judge(&unproven).1.contains("路径"));
}

#[test]
fn 路径超上限与其余正证据全部并存且缺字段时正文仍在长度预算内() {
    let worst = BridgeEvidence {
        path_usage: Some(PathUsage {
            used: u64::MAX,
            budget: u64::MAX - 1,
        }),
        deletes_blocked: Some(u64::MAX),
        fused: Some(true),
        residue_guard: Some(ResidueGuard::Blocked),
        config_matches: Some(false),
        unknown_jobs: Some(u64::MAX),
        last_exit: Some(ExitClass::OwnerStopped),
        recovery: Some(RecoveryState::ForcedMirrorRemote),
        ..BridgeEvidence::default()
    };

    let (state, detail) = judge(&worst);

    assert_eq!(state, HealthState::Degraded);
    assert!(detail.starts_with("这一轮要比对 18446744073709551615 条路径"));
    assert!(detail.chars().count() <= 200, "{}", detail.chars().count());
}
