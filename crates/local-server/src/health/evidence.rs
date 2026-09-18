//! 健康证据接口。本机 HTTP 层只消费证据，自己不去读磁盘、不查 Sync、不连远端。

use studiolo_observability::health::{
    HealthInputs, HealthReport, Instant, OwnerLiveness, RecoveryPointEvidence, WhitelistDiagnosis,
    aggregate,
};
use studiolo_observability::quota::QuotaVerdict;

/// 一次健康快照的全部证据。每一项都可缺失；缺失一律走"未知"。
#[derive(Debug, Default)]
pub struct EvidenceSnapshot {
    /// 本次聚合采用的本地时刻（Unix 秒）。
    pub now_unix_seconds: i64,
    /// 恢复点证据；`None` 表示连"有没有恢复点"都读不到。
    pub recovery_point: Option<RecoveryPointEvidence>,
    /// 恢复点年龄预算，单位秒。
    pub recovery_point_budget_seconds: u64,
    /// 各组件角色的活性回执。
    ///
    /// 字段名刻意避开写者边界词法门禁保留的前缀：`local-server` 禁止本 crate
    /// 以点号访问该前缀开头的字段，健康页也确实不该碰 canonical 写者。
    pub liveness: Vec<OwnerLiveness>,
    /// 磁盘配额结论。
    pub disk: QuotaVerdict,
    /// Sync 配额结论。
    pub sync: QuotaVerdict,
    /// 白名单诊断结论；`None` 表示诊断本身没跑成。
    pub whitelist: Option<WhitelistDiagnosis>,
}

impl EvidenceSnapshot {
    /// 什么都读不到时的快照：所有维度一律"未知"，绝不显示绿。
    #[must_use]
    pub fn unknown(now_unix_seconds: i64) -> Self {
        Self {
            now_unix_seconds,
            recovery_point: None,
            recovery_point_budget_seconds: 0,
            liveness: Vec::new(),
            disk: QuotaVerdict::Unknown,
            sync: QuotaVerdict::Unknown,
            whitelist: None,
        }
    }

    /// 聚合成只读报告。
    #[must_use]
    pub fn report(&self) -> HealthReport {
        let inputs = HealthInputs {
            now: Instant::from_unix_seconds(self.now_unix_seconds),
            recovery_point: self.recovery_point,
            recovery_point_budget_seconds: self.recovery_point_budget_seconds,
            owners: &self.liveness,
            disk: self.disk,
            sync: self.sync,
            whitelist: self.whitelist.as_ref(),
        };
        // 正文越界只可能是实现缺陷；退化成"证据全部未知"，绝不退化成绿。
        aggregate(&inputs).unwrap_or_else(|_| {
            let fallback = HealthInputs {
                now: Instant::from_unix_seconds(self.now_unix_seconds),
                recovery_point: None,
                recovery_point_budget_seconds: 0,
                owners: &[],
                disk: QuotaVerdict::Unknown,
                sync: QuotaVerdict::Unknown,
                whitelist: None,
            };
            aggregate(&fallback).unwrap_or_else(|_| HealthReport::new(Vec::new()))
        })
    }
}

/// 由可信启动层装配的证据来源。
///
/// 本 trait 只允许**读**：实现不得借此写文件、改配置或触发搬运。
pub trait HealthEvidence: Send + Sync {
    /// 取一次证据快照；读不到就返回 [`EvidenceSnapshot::unknown`]，不要伪造正常。
    fn snapshot(&self) -> EvidenceSnapshot;
}
