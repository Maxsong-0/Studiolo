//! 本地只读健康聚合。没有 IO、没有时钟、没有网络；证据全部由调用方注入。

use super::{
    AgeEvidence, BridgeEvidence, HealthReport, HealthSection, HealthState, Instant, OwnerLiveness,
    RecoveryPointEvidence, SectionDetailError, WhitelistDiagnosis, bridge_section,
};
use crate::quota::QuotaVerdict;

/// 未建立恢复点时健康页固定显示的正文；这一节永远不是绿。
pub const NO_RECOVERY_POINT: &str = "尚未建立灾难恢复点";

/// Sync 配额一节正文固定的后缀：读数是 Studiolo 侧镜像字节数，不是 Obsidian 账号用量（ADR-025 D11）。
const SYNC_QUOTA_DETAIL_SUFFIX: &str = "（非账号用量）";

/// 一次聚合的全部证据。每一项都可以是 `None`，`None` 一律走"未知"。
#[derive(Clone, Debug)]
pub struct HealthInputs<'a> {
    /// 本次聚合采用的本地时刻。
    pub now: Instant,
    /// 恢复点证据；`None` 表示连"有没有恢复点"都不知道。
    pub recovery_point: Option<RecoveryPointEvidence>,
    /// 恢复点年龄预算，单位秒；超过即降级。
    pub recovery_point_budget_seconds: u64,
    /// 各 owner 的活性证据；空表示没有任何 owner 证据。
    pub owners: &'a [OwnerLiveness],
    /// 磁盘配额结论。
    pub disk: QuotaVerdict,
    /// Sync 配额结论。
    pub sync: QuotaVerdict,
    /// 白名单诊断结论；`None` 表示诊断本身没跑成。
    pub whitelist: Option<&'a WhitelistDiagnosis>,
    /// 双向 Bridge 证据（第六节）；`None` 表示驱动平面未装配或读不到。
    pub bridge: Option<&'a BridgeEvidence>,
}

/// 聚合成健康页报告。返回 `Err` 只可能是内部正文越界，属于实现缺陷。
pub fn aggregate(inputs: &HealthInputs<'_>) -> Result<HealthReport, SectionDetailError> {
    Ok(HealthReport::new(vec![
        recovery_point_section(inputs)?,
        owners_section(inputs)?,
        quota_section("disk-quota", "磁盘配额", inputs.disk, "")?,
        quota_section(
            "sync-quota",
            "Sync 配额",
            inputs.sync,
            SYNC_QUOTA_DETAIL_SUFFIX,
        )?,
        whitelist_section(inputs.whitelist)?,
        bridge_section(inputs.bridge)?,
    ]))
}

fn recovery_point_section(inputs: &HealthInputs<'_>) -> Result<HealthSection, SectionDetailError> {
    let (state, detail) = match inputs.recovery_point {
        None => (
            HealthState::Unknown,
            "恢复点证据不可读，无法判断灾难恢复能力".to_owned(),
        ),
        Some(RecoveryPointEvidence::NotEstablished) => {
            (HealthState::Unknown, NO_RECOVERY_POINT.to_owned())
        }
        Some(RecoveryPointEvidence::Established { content_cutoff }) => {
            match inputs.now.age_since(content_cutoff) {
                AgeEvidence::ClockRolledBack => (
                    HealthState::Unknown,
                    "本地时钟早于恢复点内容截止时刻，时间证据不可信".to_owned(),
                ),
                AgeEvidence::Untrusted => (
                    HealthState::Unknown,
                    "恢复点年龄超出可判断范围，时间证据不可信".to_owned(),
                ),
                AgeEvidence::Known(age) if age > inputs.recovery_point_budget_seconds => (
                    HealthState::Degraded,
                    format!("最新恢复点已有 {age} 秒，超过约定预算"),
                ),
                AgeEvidence::Known(age) => (
                    HealthState::Normal,
                    format!("最新恢复点 {age} 秒前的内容，已在预算内"),
                ),
            }
        }
    };
    HealthSection::new("recovery-point", "恢复点年龄", state, detail)
}

fn owners_section(inputs: &HealthInputs<'_>) -> Result<HealthSection, SectionDetailError> {
    if inputs.owners.is_empty() {
        return HealthSection::new(
            "owner-liveness",
            "各 owner 活性",
            HealthState::Unknown,
            "没有任何 owner 活性证据",
        );
    }
    let mut unknown = 0_usize;
    let mut stale = 0_usize;
    let mut fresh = 0_usize;
    for owner in inputs.owners {
        let Some(receipt) = owner.last_receipt else {
            unknown += 1;
            continue;
        };
        if owner.expected_period_seconds == 0 {
            unknown += 1;
            continue;
        }
        match inputs.now.age_since(receipt) {
            AgeEvidence::Known(age) if age <= owner.expected_period_seconds => fresh += 1,
            AgeEvidence::Known(_) => stale += 1,
            AgeEvidence::ClockRolledBack | AgeEvidence::Untrusted => unknown += 1,
        }
    }
    let state = if stale > 0 {
        HealthState::Degraded
    } else if unknown > 0 {
        HealthState::Unknown
    } else {
        HealthState::Normal
    };
    HealthSection::new(
        "owner-liveness",
        "各 owner 活性",
        state,
        format!("活性回执正常 {fresh} 个，超期 {stale} 个，证据不足 {unknown} 个"),
    )
}

/// 配额一节：三态文案固定，`detail_suffix` 原样拼在文案之后（磁盘传空，Sync 传「非账号用量」）。
fn quota_section(
    code: &'static str,
    title: &'static str,
    verdict: QuotaVerdict,
    detail_suffix: &str,
) -> Result<HealthSection, SectionDetailError> {
    let detail = match verdict {
        QuotaVerdict::Normal => "占用在预警水位以内",
        QuotaVerdict::Warning => "占用已越过预警水位，尚未越过上限",
        QuotaVerdict::Exceeded => "占用已越过上限，对应写入路径已阻断",
        QuotaVerdict::Unknown => "占用读数不可得，无法判断",
    };
    HealthSection::new(
        code,
        title,
        verdict.health(),
        format!("{detail}{detail_suffix}"),
    )
}

fn whitelist_section(
    diagnosis: Option<&WhitelistDiagnosis>,
) -> Result<HealthSection, SectionDetailError> {
    let (state, detail) = match diagnosis {
        None => (
            HealthState::Unknown,
            "白名单诊断未能执行，无法判断一致性".to_owned(),
        ),
        Some(diagnosis) if diagnosis.consistent() => (
            HealthState::Normal,
            format!("检查 {} 条登记，未发现冲突", diagnosis.examined()),
        ),
        Some(diagnosis) => (
            HealthState::Degraded,
            format!(
                "检查 {} 条登记，发现 {} 条冲突，需人工改配置",
                diagnosis.examined(),
                diagnosis.findings().len()
            ),
        ),
    };
    HealthSection::new("whitelist-diagnosis", "白名单诊断", state, detail)
}
