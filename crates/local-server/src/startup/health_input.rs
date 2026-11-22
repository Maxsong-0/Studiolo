//! 组合根装配健康证据来源：**只如实反映本次装配真正拿到了哪些证据**。
//!
//! 健康页的每一节都必须有可指认的证据来源才敢显示"正常"。目前有两个真实证据源：双向 Bridge 平面
//! （第六节、`sync-transport` 活性、Sync 配额读数，全部经 [`BridgeHealthProbe`]）和备份生产者
//! （「恢复点年龄」「备份作业」「备份仓库检查」三节，经 [`BackupJobsProbe`] 只读原账本里的备份事件）；
//! 其余各节没接上的原因逐条写在 [`AssembledEvidence::snapshot`] 上，一律给出"未知"。
//! **绝不为了页面好看而假装正常**：`EvidenceSnapshot::unknown` 的每一节都会被只读聚合
//! 判成 `Unknown`，平面没装配时整体因此永远不是绿。
//!
//! 本 Module 不读磁盘、不连网络、不读环境变量，也不持有任何配置或秘密；
//! 外部输入只有本地墙钟（每次取快照时读）和两个探针各自的只读回答。

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_local_server::backup::{BackupJobsEvidence, BackupJobsProbe, RepositoryCheckEvidence};
use studiolo_local_server::health::{EvidenceSnapshot, HealthEvidence};
use studiolo_local_server::sync_plane::health::BridgeHealthProbe;
use studiolo_observability::health::{Instant, RecoveryPointEvidence};
use studiolo_observability::quota::{QuotaVerdict, SyncQuota, SyncUsage};

/// 本次装配可得的健康证据。
///
/// 只要哪天真有证据源接进组合根，就往这里加字段，加一个字段就多一节能显示真实结论；
/// 没加的一律留在"未知"。目前有双向 Bridge 平面和备份生产者（各自装配了才有）。
pub(crate) struct AssembledEvidence {
    /// 双向 Bridge 平面的健康探针；平面未装配即 `None`。
    bridge: Option<Arc<dyn BridgeHealthProbe>>,
    /// 备份生产者的作业探针；启动时没有给备份配置即 `None`。
    backup: Option<Arc<dyn BackupJobsProbe>>,
}

impl HealthEvidence for AssembledEvidence {
    /// 取一次证据快照。
    ///
    /// 逐节说明当前为什么是"未知"：
    ///
    /// - **恢复点年龄**：和「备份作业」用同一次读到的作业事实，见 [`recovery_point`]。
    ///   没装配生产者报「备份没有启用」，这一次没读到报「证据不可读」，读到了但还没有一次
    ///   成功作业报 `NO_RECOVERY_POINT`，都是未知；有了就按成功作业准入里的内容截止算年龄、
    ///   对主人批准的 RPO 判正常或降级。不读 worker 的状态库：那是 worker 账号的目录。
    /// - **各 owner 活性**：只有 `sync-transport` 一个 owner 经探针给回执（平面装配了才有）；
    ///   备份 Worker 的回执仍没接线，所以它不在列表里，而不是被算成"正常"。
    /// - **磁盘配额**：剩余空间只能由持文件系统权限的 Broker 读；
    ///   `vault-broker` 目前没有对外公开的剩余字节查询，HTTP 层自己更不许读盘。
    /// - **Sync 配额**：读数是探针给的镜像字节数与对象数（不是账号用量），上限是审批文档里
    ///   人工抄录的账号套餐上限、预警取七成；平面未装配或人工配额缺失就是"未知"。
    /// - **白名单诊断**：诊断需要把 `SyncScope` / `file_operations` /
    ///   `human_content` 摊平成同一份快照，而 `host-config` 目前只公开
    ///   `observation_selection`（保护前缀与生成区已被合并成一张 `excluded` 表），
    ///   没有公开人侧可写范围与 `human_content` 登记。用半份快照去诊断会凭空
    ///   造出"搬运范围覆盖保护前缀"这类假冲突，比报未知更坏，所以报未知。
    /// - **双向 Bridge**：平面装配了才有探针。`probe.bridge()` 进第六节，`probe.liveness()`
    ///   是 `sync-transport` 这一个 owner 的活性回执，`probe.sync_usage()` 配合
    ///   `probe.manual_quota()` 经 [`sync_quota_verdict`] 算 Sync 配额。
    /// - **备份作业**：生产者装配了才有探针，它从启动保留的原账本读备份事件；没装配报「没有启用」，
    ///   装配了但这一次没读到（账本没读完、本地时钟不可用）报「没读到」，都是未知。
    fn snapshot(&self) -> EvidenceSnapshot {
        let mut snapshot = EvidenceSnapshot::unknown(now_unix_seconds());
        if let Some(probe) = &self.bridge {
            snapshot.bridge = Some(probe.bridge());
            snapshot.liveness = vec![probe.liveness()];
            snapshot.manual_sync_quota = probe.manual_quota();
            snapshot.sync = sync_quota_verdict(snapshot.manual_sync_quota, probe.sync_usage());
        }
        snapshot.backup_jobs = match &self.backup {
            None => BackupJobsEvidence::Disabled,
            Some(probe) => now_unix_millis()
                .and_then(|now| probe.jobs(now))
                .map_or(BackupJobsEvidence::Unreadable, BackupJobsEvidence::Read),
        };
        let (recovery_point, budget_seconds) = recovery_point(&snapshot.backup_jobs);
        snapshot.recovery_point = recovery_point;
        snapshot.recovery_point_budget_seconds = budget_seconds;
        snapshot.backup_checks = match &self.backup {
            None => RepositoryCheckEvidence::Disabled,
            Some(probe) => now_unix_millis()
                .and_then(|now| probe.repository_checks(now))
                .map_or(RepositoryCheckEvidence::Unreadable, |checks| {
                    RepositoryCheckEvidence::Read(Box::new(checks))
                }),
        };
        snapshot
    }
}

/// 构造本次装配的证据来源。
pub(crate) fn source(
    bridge: Option<Arc<dyn BridgeHealthProbe>>,
    backup: Option<Arc<dyn BackupJobsProbe>>,
) -> Arc<dyn HealthEvidence> {
    Arc::new(AssembledEvidence { bridge, backup })
}

/// 「恢复点年龄」一节的证据和预算（秒），从「备份作业」同一次读到的事实推出。
///
/// 聚合按秒算年龄，毫秒换秒向下取整，年龄误差在 1 秒以内；预算向下取整只会更严。
fn recovery_point(jobs: &BackupJobsEvidence) -> (Option<RecoveryPointEvidence>, u64) {
    match jobs {
        BackupJobsEvidence::Disabled => (Some(RecoveryPointEvidence::Disabled), 0),
        BackupJobsEvidence::Unreadable => (None, 0),
        BackupJobsEvidence::Read(jobs) => {
            let evidence = match jobs.recovery_point {
                None => RecoveryPointEvidence::NotEstablished,
                Some(mark) => RecoveryPointEvidence::Established {
                    content_cutoff: Instant::from_unix_seconds(
                        mark.content_cutoff_ms.div_euclid(1_000),
                    ),
                },
            };
            (Some(evidence), jobs.recovery_point_budget_ms / 1_000)
        }
    }
}

/// `SyncScope` 固定的范围内对象上限（`host-config` 的 `sync_scope/union.rs::MAX_PATHS`，未公开）。
const SYNC_SCOPE_MAX_OBJECTS: u64 = 4096;

/// 预警水位：上限的七成，整数截断；乘法越界即 `None`。
fn seven_tenths(limit: u64) -> Option<u64> {
    limit.checked_mul(7).map(|value| value / 10)
}

/// 用镜像读数对人工配额下结论（ADR-025 D11）：字节上限＝账号套餐上限、对象上限＝`SyncScope`
/// 固定的 4096，预警各取七成。人工配额缺失、水位不自洽或读数缺失一律未知，不算正常。
fn sync_quota_verdict(quota: Option<ManualQuota>, usage: Option<SyncUsage>) -> QuotaVerdict {
    let Some(quota) = quota else {
        return QuotaVerdict::Unknown;
    };
    let (Some(warn_objects), Some(warn_bytes)) = (
        seven_tenths(SYNC_SCOPE_MAX_OBJECTS),
        seven_tenths(quota.account_limit_bytes),
    ) else {
        return QuotaVerdict::Unknown;
    };
    SyncQuota::new(
        SYNC_SCOPE_MAX_OBJECTS,
        quota.account_limit_bytes,
        warn_objects,
        warn_bytes,
    )
    .map_or(QuotaVerdict::Unknown, |limits| limits.evaluate(usage))
}

/// 本地墙钟的 Unix 秒。
///
/// 时钟早于纪元或超出 `i64` 时返回 `None` 对应的兜底 0：聚合层对"时间证据"
/// 本来就有独立的回退/越界判定，绝不会因为一个可疑时刻而把某节判成正常。
fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or_default()
}

/// 本地墙钟的 Unix 毫秒；早于纪元或越界为 `None`，备份作业一节就报没读到，不拿兜底值去判断卡住和逾期。
fn now_unix_millis() -> Option<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
}

#[cfg(test)]
mod tests {
    use super::{AssembledEvidence, now_unix_millis, now_unix_seconds, source, sync_quota_verdict};
    use std::sync::Arc;
    use studiolo_host_config::sync_transport::ManualQuota;
    use studiolo_local_server::backup::{
        BackupJobs, BackupJobsProbe, JobMark, RecoveryMark, RepositoryChecks,
    };
    use studiolo_local_server::health::{HealthBody, HealthEvidence, SectionBody};
    use studiolo_local_server::sync_plane::health::{BridgeHealthProbe, OWNER_LABEL};
    use studiolo_observability::health::{
        BACKUP_DISABLED, BatchStamp, BridgeEvidence, ExitClass, Instant, ModeLabel,
        NO_RECOVERY_POINT, OwnerLabel, OwnerLiveness, RecoveryState, ResidueGuard,
    };
    use studiolo_observability::quota::{QuotaVerdict, SyncUsage};

    const QUOTA: ManualQuota = ManualQuota {
        account_limit_bytes: 10_000,
        used_bytes_manual: 1_000,
        recorded_at_unix: 1_700_000_000,
    };

    /// 全字段齐备、无异常、读数在预警内的探针；时刻按墙钟取，活性回执才不会超期。
    struct FakeProbe;

    impl BridgeHealthProbe for FakeProbe {
        fn bridge(&self) -> BridgeEvidence {
            BridgeEvidence {
                last_batch: Some(BatchStamp {
                    sequence: 3,
                    unix_seconds: now_unix_seconds() - 30,
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
                config_matches: Some(true),
                deletes_blocked: Some(0),
            }
        }

        fn liveness(&self) -> OwnerLiveness {
            OwnerLiveness {
                owner: OwnerLabel::parse(OWNER_LABEL).expect("固定标签必须合法"),
                last_receipt: Some(Instant::from_unix_seconds(now_unix_seconds() - 10)),
                expected_period_seconds: 600,
            }
        }

        fn sync_usage(&self) -> Option<SyncUsage> {
            Some(SyncUsage {
                objects: 12,
                bytes: 4_096,
            })
        }

        fn manual_quota(&self) -> Option<ManualQuota> {
            Some(QUOTA)
        }
    }

    /// 生产者探针的替身：`readable` 为假表示这一次没读完账本；读得到时最近一次成功在一分钟前，
    /// 首轮抽查里第 1 组一小时前读干净；`cutoff_age_ms` 是恢复点内容截止离现在多久，`None` 是还没有恢复点。
    /// RPO 是 15 分钟。
    struct FakeBackup {
        readable: bool,
        cutoff_age_ms: Option<i64>,
    }

    /// 读得到、恢复点内容截止在 100 秒前的替身。
    const FRESH: FakeBackup = FakeBackup {
        readable: true,
        cutoff_age_ms: Some(100_000),
    };

    impl BackupJobsProbe for FakeBackup {
        fn jobs(&self, now_ms: i64) -> Option<BackupJobs> {
            self.readable.then_some(BackupJobs {
                now_ms,
                serving_since_ms: Some(now_ms - 3_600_000),
                first_attempt_ms: Some(now_ms - 3_000_000),
                last_success: Some(JobMark {
                    sequence: 7,
                    at_ms: now_ms - 60_000,
                }),
                last_failure: None,
                latest_failed: false,
                outstanding: None,
                stuck_after_ms: 86_400_000,
                overdue_after_ms: 7_200_000,
                recovery_point: self.cutoff_age_ms.map(|age| RecoveryMark {
                    sequence: 7,
                    content_cutoff_ms: now_ms - age,
                }),
                recovery_point_budget_ms: 900_000,
            })
        }

        fn repository_checks(&self, now_ms: i64) -> Option<RepositoryChecks> {
            let mut last_clean_ms = [None; 30];
            last_clean_ms[0] = Some(now_ms - 3_600_000);
            self.readable.then_some(RepositoryChecks {
                now_ms,
                anchor_ms: Some(now_ms - 86_400_000),
                repository_since_ms: Some(now_ms - 3_600_000),
                latest_record_ms: Some(now_ms - 3_600_000),
                running: None,
                latest: None,
                last_clean_ms,
                problems: Vec::new(),
            })
        }
    }

    fn section<'a>(body: &'a HealthBody, code: &str) -> &'a SectionBody {
        body.sections
            .iter()
            .find(|section| section.code == code)
            .expect("响应必须包含该节")
    }

    #[test]
    fn 装配了探针时第六节传输活性与sync配额三节都来自探针() {
        let probe: Arc<dyn BridgeHealthProbe> = Arc::new(FakeProbe);
        let evidence = AssembledEvidence {
            bridge: Some(probe),
            backup: None,
        };

        let body = HealthBody::from_report(&evidence.snapshot().report());

        let bridge = section(&body, "sync-bridge");
        assert_eq!(bridge.state, "normal");
        assert!(bridge.detail.starts_with("第 3 批于 "));
        let liveness = section(&body, "owner-liveness");
        assert_eq!(liveness.state, "normal");
        assert_eq!(
            liveness.detail,
            "活性回执正常 1 个，超期 0 个，证据不足 0 个"
        );
        let sync = section(&body, "sync-quota");
        assert_eq!(sync.state, "normal");
        assert_eq!(sync.detail, "占用在预警水位以内（非账号用量）");
        assert_eq!(section(&body, "disk-quota").state, "unknown");
        assert_eq!(body.overall, "unknown", "其余几节仍无证据源");
    }

    /// 只装配了生产者时的响应体。
    fn backup_only(probe: FakeBackup) -> HealthBody {
        let probe: Arc<dyn BackupJobsProbe> = Arc::new(probe);
        HealthBody::from_snapshot(
            &AssembledEvidence {
                bridge: None,
                backup: Some(probe),
            }
            .snapshot(),
        )
    }

    #[test]
    fn 有成功作业时恢复点一节按准入里的内容截止对rpo判正常或降级() {
        let body = backup_only(FRESH);
        let recovery = section(&body, "recovery-point");
        assert_eq!(recovery.state, "normal");
        // 内容截止在 100 秒前；快照的秒先读、作业事实的毫秒后读，跨了一秒时年龄少算 1 秒。
        assert!(
            [
                "最新恢复点 100 秒前的内容，已在预算内",
                "最新恢复点 99 秒前的内容，已在预算内"
            ]
            .contains(&recovery.detail.as_str()),
            "{}",
            recovery.detail
        );

        let body = backup_only(FakeBackup {
            readable: true,
            cutoff_age_ms: Some(901_000),
        });
        let recovery = section(&body, "recovery-point");
        assert_eq!(recovery.state, "degraded", "内容截止早于 RPO 15 分钟");
        assert!(recovery.detail.ends_with("超过约定预算"));
    }

    #[test]
    fn 读到账本但还没有成功作业时恢复点一节固定显示尚未建立() {
        let body = backup_only(FakeBackup {
            readable: true,
            cutoff_age_ms: None,
        });
        let recovery = section(&body, "recovery-point");
        assert_eq!(recovery.state, "unknown");
        assert_eq!(recovery.detail, NO_RECOVERY_POINT);
    }

    #[test]
    fn 没读到账本时恢复点一节说证据不可读没启用时说没启用() {
        let body = backup_only(FakeBackup {
            readable: false,
            cutoff_age_ms: Some(100_000),
        });
        let recovery = section(&body, "recovery-point");
        assert_eq!(recovery.state, "unknown");
        assert!(recovery.detail.starts_with("恢复点证据不可读"));

        let body = HealthBody::from_snapshot(&source(None, None).snapshot());
        let recovery = section(&body, "recovery-point");
        assert_eq!(recovery.state, "unknown");
        assert_eq!(recovery.detail, BACKUP_DISABLED);
    }

    #[test]
    fn 装配了生产者时备份作业一节和明细来自同一次读账本() {
        let probe: Arc<dyn BackupJobsProbe> = Arc::new(FRESH);
        let evidence = AssembledEvidence {
            bridge: None,
            backup: Some(probe),
        };

        let body = HealthBody::from_snapshot(&evidence.snapshot());

        let jobs = section(&body, "backup-jobs");
        assert_eq!(jobs.state, "normal");
        assert_eq!(jobs.detail, "最近一次成功是 1 分钟前的第 7 号");
        let checks = section(&body, "backup-repository-check");
        assert_eq!(checks.state, "normal");
        assert_eq!(
            checks.detail,
            "最近一次干净的检查是 1 小时前；首轮还在读，30 组里读过 1 组"
        );
        let value = serde_json::to_value(&body).expect("响应必须可序列化");
        assert_eq!(value["backup"]["verdict"], "healthy");
        assert_eq!(value["backup"]["last_success"]["sequence"], "7");
        assert_eq!(value["backup"]["overdue_after_seconds"], "7200");
        assert_eq!(body.overall, "unknown", "别的节没有证据源，整体仍不是绿");
    }

    #[test]
    fn 生产者这一次没读到或者没启用时备份作业一节报未知而不是正常() {
        let unread: Arc<dyn BackupJobsProbe> = Arc::new(FakeBackup {
            readable: false,
            cutoff_age_ms: None,
        });
        let body = HealthBody::from_snapshot(
            &AssembledEvidence {
                bridge: None,
                backup: Some(unread),
            }
            .snapshot(),
        );
        let jobs = section(&body, "backup-jobs");
        assert_eq!(jobs.state, "unknown");
        assert_eq!(jobs.detail, "这一次没读到账本里的备份事件，作业状态未知");
        assert!(body.backup.is_none());
        let checks = section(&body, "backup-repository-check");
        assert_eq!(checks.state, "unknown");
        assert_eq!(checks.detail, "这一次没读到账本里的仓库检查事件，状态未知");

        let body = HealthBody::from_snapshot(
            &AssembledEvidence {
                bridge: None,
                backup: None,
            }
            .snapshot(),
        );
        let jobs = section(&body, "backup-jobs");
        assert_eq!(jobs.state, "unknown");
        assert!(jobs.detail.starts_with("备份生产者没有启用"));
        assert!(body.backup.is_none());
        let checks = section(&body, "backup-repository-check");
        assert_eq!(checks.state, "unknown");
        assert!(checks.detail.starts_with("备份生产者没有启用"));
    }

    #[test]
    fn sync配额按镜像字节数对人工配额上限七成预警() {
        let verdict = |bytes| {
            let usage = SyncUsage { objects: 1, bytes };
            sync_quota_verdict(Some(QUOTA), Some(usage))
        };

        assert_eq!(sync_quota_verdict(None, None), QuotaVerdict::Unknown);
        assert_eq!(sync_quota_verdict(Some(QUOTA), None), QuotaVerdict::Unknown);
        assert_eq!(verdict(6_999), QuotaVerdict::Normal);
        assert_eq!(verdict(7_000), QuotaVerdict::Warning);
        assert_eq!(verdict(10_000), QuotaVerdict::Warning);
        assert_eq!(verdict(10_001), QuotaVerdict::Exceeded);
    }

    #[test]
    fn sync配额对象维度取范围固定上限4096与其七成() {
        let verdict = |objects| {
            let usage = SyncUsage { objects, bytes: 1 };
            sync_quota_verdict(Some(QUOTA), Some(usage))
        };

        assert_eq!(verdict(2_866), QuotaVerdict::Normal);
        assert_eq!(verdict(2_867), QuotaVerdict::Warning);
        assert_eq!(verdict(4_096), QuotaVerdict::Warning);
        assert_eq!(verdict(4_097), QuotaVerdict::Exceeded);
    }

    #[test]
    fn 人工配额水位不自洽或越界时sync配额判未知而不是判正常() {
        let usage = SyncUsage {
            objects: 1,
            bytes: 1,
        };
        let tiny = ManualQuota {
            account_limit_bytes: 1,
            ..QUOTA
        };
        let huge = ManualQuota {
            account_limit_bytes: u64::MAX,
            ..QUOTA
        };

        assert_eq!(
            sync_quota_verdict(Some(tiny), Some(usage)),
            QuotaVerdict::Unknown
        );
        assert_eq!(
            sync_quota_verdict(Some(huge), Some(usage)),
            QuotaVerdict::Unknown
        );
    }

    #[test]
    fn 组合根尚未接上任何证据源因而健康页整体判未知而不是判正常() {
        let body = HealthBody::from_report(&source(None, None).snapshot().report());

        assert_eq!(body.overall, "unknown");
        assert!(!body.evidence_complete);
    }

    #[test]
    fn 九节结论一节不少且全部标为未知() {
        let body = HealthBody::from_report(&source(None, None).snapshot().report());

        assert_eq!(body.sections.len(), 9);
        assert!(
            body.sections
                .iter()
                .all(|section| section.state == "unknown"),
            "任何一节都不许在没有证据的情况下报正常"
        );
    }

    #[test]
    fn 取到的本地时刻落在纪元之后因而不是兜底零值() {
        assert!(now_unix_seconds() > 1_700_000_000);
        assert!(now_unix_millis().is_some_and(|now| now > 1_700_000_000_000));
    }

    #[test]
    fn 同一个证据来源连续取两次都给出同样的未知结论() {
        let evidence = source(None, None);

        let first = HealthBody::from_report(&evidence.snapshot().report());
        let second = HealthBody::from_report(&evidence.snapshot().report());

        assert_eq!(first.overall, "unknown");
        assert_eq!(second.overall, "unknown");
        assert_eq!(first.sections.len(), second.sections.len());
    }
}
