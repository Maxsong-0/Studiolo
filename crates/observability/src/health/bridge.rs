//! 健康页第六节「双向 Bridge」的证据 DTO 与判定（ADR-025 D11）。
//!
//! 每个字段都是 `Option`：装配层拿不到就填 `None`，本节因此一律「未知」而不是「正常」。
//! 这里没有 IO，也不读时钟；时刻由装配层注入。

use super::{HealthSection, HealthState, SectionDetailError};

/// 本节的稳定机器码。
pub const BRIDGE_SECTION_CODE: &str = "sync-bridge";
/// 本节的中文标题。
pub const BRIDGE_SECTION_TITLE: &str = "双向 Bridge";
/// 排除项守卫阻断时的固定中文原因（ADR-025 D10）。
pub const RESIDUE_BLOCKED: &str = "排除项清除前有镜像残留";
/// headless 本地配置与审批文档不一致时的固定中文原因（设计文档第 2 节第 3 步）。
pub const CONFIG_MISMATCH: &str = "headless 配置与审批文档不一致，本轮未同步";

/// 最近一批的序号与时刻。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchStamp {
    /// Bridge 计划序号。
    pub sequence: u64,
    /// 该批闭合（或停住）的 Unix 秒。
    pub unix_seconds: i64,
}

/// 上次 headless 运行的退出分类；只按退出码、停止原因与静止证据分类，不解析 stderr。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitClass {
    /// 退出码 0 且静止已证明。
    Clean,
    /// 退出码非零（含 rc=1 残留锁）。
    NonZero,
    /// 被信号终止。
    Signal,
    /// 超过 wall deadline 被 owner 停止。
    DeadlineExceeded,
    /// owner 关停（`studiolo-human` 收到停止信号）时被 `request_stop` 停下；非干净，照样降级。
    OwnerStopped,
    /// 静止或退出结果未取得。
    Unconfirmed,
}

/// 非正常退出恢复协议（ADR-025 D9）当前所处状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryState {
    /// 上次运行已干净回收，按审批模式运行。
    Idle,
    /// 上次未干净回收，正强制 `mirror-remote` 恢复一次。
    ForcedMirrorRemote,
    /// 处于退避窗口，下一次 RunOnce 尚未到时。
    Backoff,
}

/// 排除项守卫（ADR-025 D10）状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidueGuard {
    /// 当前排除集合与上次核验一致，或残留已清。
    Clear,
    /// 有已移出排除集合的文件夹在镜像里仍有残留；RunOnce 被拒。
    Blocked,
}

/// 审批的镜像模式标签；健康页只显示，不据此授权。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeLabel {
    /// 引导期只下载。
    MirrorRemote,
    /// P6 双向。
    Bidirectional,
}

/// 第六节的全部证据；`None` 一律走「未知」。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeEvidence {
    /// 最近一批序号与时刻。
    pub last_batch: Option<BatchStamp>,
    /// 删除熔断是否仍生效。
    pub fused: Option<bool>,
    /// 终态未知的动作数。
    pub unknown_jobs: Option<u64>,
    /// 当前静止证据是否仍有效。
    pub quiescence_valid: Option<bool>,
    /// 冲突副本数：最近一次完整观察的两侧副本路径数，加持久 conflict_stamps 的当前条数。
    pub conflict_copies: Option<u64>,
    /// Studiolo 侧镜像字节数；不是账号用量。
    pub mirror_bytes: Option<u64>,
    /// 上次运行退出分类。
    pub last_exit: Option<ExitClass>,
    /// 恢复协议状态。
    pub recovery: Option<RecoveryState>,
    /// 排除项守卫状态。
    pub residue_guard: Option<ResidueGuard>,
    /// 当前生效的镜像模式。
    pub mode: Option<ModeLabel>,
    /// headless 本地配置（模式／排除项／文件类型）是否与审批文档一致；不一致本轮不跑 RunOnce。
    pub config_matches: Option<bool>,
}

/// 聚合第六节；缺证据一律未知。
///
/// 判定顺序固定：缺字段 → 删除熔断 → 排除项残留 → 配置不符 → 终态未知 → 非干净退出 → 恢复协议进行中 →
/// 静止证据过期 → 正常。驱动平面未装配（默认关闭）时也是「未知」：与恢复点、白名单两节同一口径，
/// 没启用的能力不显示绿。
///
/// # 错误
/// 只可能是内部正文越界，属实现缺陷。
pub fn bridge_section(
    evidence: Option<&BridgeEvidence>,
) -> Result<HealthSection, SectionDetailError> {
    let (state, detail) = match evidence {
        None => (
            HealthState::Unknown,
            "驱动平面未装配或证据不可读，无法判断双向同步状态".to_owned(),
        ),
        Some(evidence) => judge(evidence),
    };
    HealthSection::new(BRIDGE_SECTION_CODE, BRIDGE_SECTION_TITLE, state, detail)
}

/// 全字段齐备的证据；由 [`complete`] 从 `Option` 逐项取出。
struct Complete {
    last_batch: BatchStamp,
    fused: bool,
    unknown_jobs: u64,
    quiescence_valid: bool,
    conflict_copies: u64,
    mirror_bytes: u64,
    last_exit: ExitClass,
    recovery: RecoveryState,
    residue_guard: ResidueGuard,
    mode: ModeLabel,
    config_matches: bool,
}

/// 第一个缺失字段的中文名作为 `Err`，正文里只报字段名，不报数值。
fn complete(evidence: &BridgeEvidence) -> Result<Complete, &'static str> {
    Ok(Complete {
        last_batch: evidence.last_batch.ok_or("最近一批")?,
        fused: evidence.fused.ok_or("删除熔断")?,
        unknown_jobs: evidence.unknown_jobs.ok_or("终态未知动作数")?,
        quiescence_valid: evidence.quiescence_valid.ok_or("静止证据")?,
        conflict_copies: evidence.conflict_copies.ok_or("冲突副本数")?,
        mirror_bytes: evidence.mirror_bytes.ok_or("镜像字节数")?,
        last_exit: evidence.last_exit.ok_or("退出分类")?,
        recovery: evidence.recovery.ok_or("恢复协议状态")?,
        residue_guard: evidence.residue_guard.ok_or("排除项守卫")?,
        mode: evidence.mode.ok_or("镜像模式")?,
        config_matches: evidence.config_matches.ok_or("配置核对")?,
    })
}

fn judge(evidence: &BridgeEvidence) -> (HealthState, String) {
    let full = match complete(evidence) {
        Ok(full) => full,
        Err(missing) => {
            let mut detail = format!("Bridge 证据不完整：缺{missing}");
            if let Some(exit) = evidence.last_exit.filter(|exit| *exit != ExitClass::Clean) {
                detail.push_str(&format!("；上次 headless 运行{}", exit.label()));
            }
            return (HealthState::Unknown, detail);
        }
    };
    if full.fused {
        return (
            HealthState::Degraded,
            "删除熔断生效，本批与后续批次全部阻断".to_owned(),
        );
    }
    if full.residue_guard == ResidueGuard::Blocked {
        return (HealthState::Degraded, RESIDUE_BLOCKED.to_owned());
    }
    if !full.config_matches {
        return (HealthState::Degraded, CONFIG_MISMATCH.to_owned());
    }
    if full.unknown_jobs > 0 {
        return (
            HealthState::Degraded,
            format!("有 {} 条动作终态未知，等待原键核对", full.unknown_jobs),
        );
    }
    if full.last_exit != ExitClass::Clean {
        return (
            HealthState::Degraded,
            format!("上次 headless 运行{}", full.last_exit.label()),
        );
    }
    if full.recovery != RecoveryState::Idle {
        return (
            HealthState::Degraded,
            format!("恢复协议进行中：{}", full.recovery.label()),
        );
    }
    if !full.quiescence_valid {
        return (
            HealthState::Unknown,
            "静止证据已过期，等待下一轮重新取得".to_owned(),
        );
    }
    (
        HealthState::Normal,
        format!(
            "第 {} 批于 {} 闭合；冲突副本 {}；镜像 {} 字节（非账号用量）；模式 {}",
            full.last_batch.sequence,
            full.last_batch.unix_seconds,
            full.conflict_copies,
            full.mirror_bytes,
            full.mode.label()
        ),
    )
}

impl ExitClass {
    /// 健康页正文里的固定中文。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Clean => "干净退出",
            Self::NonZero => "以非零退出码结束",
            Self::Signal => "被信号终止",
            Self::DeadlineExceeded => "超过时限被停止",
            Self::OwnerStopped => "在 owner 关停时被停止",
            Self::Unconfirmed => "退出结果未确认",
        }
    }
}

impl RecoveryState {
    /// 健康页正文里的固定中文。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "空闲",
            Self::ForcedMirrorRemote => "强制只下载恢复一次",
            Self::Backoff => "退避等待",
        }
    }
}

impl ModeLabel {
    /// 与审批文档 `headless_mode` 相同的取值。
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::MirrorRemote => "mirror-remote",
            Self::Bidirectional => "bidirectional",
        }
    }
}
