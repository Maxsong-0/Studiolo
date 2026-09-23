//! 有界的 attempt 预算与单调递增的进度。
use serde::{Deserialize, Serialize};

/// ADR-018 规定的工作量与耗时阈值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunBudget {
    /// 第一档节奏提醒（pacing）的阈值。
    pub pacing_tools: u64,
    /// 工具调用数达到此阈值即开始准备回执。
    pub wrapup_tools: u64,
    /// 达到此上限后不再允许调用任何工具。
    pub max_tools: u64,
    /// 运行满这么多秒后开始准备回执。
    pub wrapup_seconds: u64,
    /// 运行满这么多秒后请求取消。
    pub max_seconds: u64,
}
impl Default for RunBudget {
    fn default() -> Self {
        Self {
            pacing_tools: 24,
            wrapup_tools: 120,
            max_tools: 160,
            wrapup_seconds: 1500,
            max_seconds: 2100,
        }
    }
}
/// 发给生命周期持有方的预算状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPhase {
    /// 正常执行。
    #[default]
    Working,
    /// 应发出节奏提醒。
    Pacing,
    /// 应开始准备回执。
    Wrapup,
    /// 必须拒绝后续的工具调用。
    Exhausted,
}
/// 由 CLI 协议事件推导出的单调观测值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerProgress {
    /// 已开始或已完成的工具调用数；同一事件身份只计一次。
    pub tool_calls: u64,
    /// 文件变更事件数；同一事件身份只计一次。
    pub edits: u64,
    /// 观测时刻的预算阶段。
    pub phase: BudgetPhase,
}
impl RunBudget {
    /// 校验预算：阈值顺序倒置或时长无上限即判为无效。
    #[must_use]
    pub fn valid(&self) -> bool {
        self.pacing_tools > 0
            && self.pacing_tools < self.wrapup_tools
            && self.wrapup_tools < self.max_tools
            && self.max_tools <= 10000
            && self.wrapup_seconds > 0
            && self.wrapup_seconds < self.max_seconds
            && self.max_seconds <= 86400
    }
    /// 由强制执行的两个维度（工具调用数、已用秒数）计算单调的预算阶段。
    #[must_use]
    pub fn phase(&self, tools: u64, seconds: u64) -> BudgetPhase {
        if tools >= self.max_tools || seconds >= self.max_seconds {
            BudgetPhase::Exhausted
        } else if tools >= self.wrapup_tools || seconds >= self.wrapup_seconds {
            BudgetPhase::Wrapup
        } else if tools >= self.pacing_tools {
            BudgetPhase::Pacing
        } else {
            BudgetPhase::Working
        }
    }
}
