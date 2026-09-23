//! 有界排空计数：只保存计数与固定分类，超限即停止，绝不截断后按成功解析。

use super::ContainmentGap;
use crate::owned_run::{ResourceCounts, RunBudget};

/// 两个输出流。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// 标准输出。
    Stdout,
    /// 标准错误。
    Stderr,
}

/// 排空判定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainVerdict {
    /// 仍在预算内。
    Continue,
    /// 已超限；必须停止本次运行，并按超预算处理。
    OverBudget,
}

/// 持续排空的计数器；只累加字节数，不保存任何正文。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputDrain {
    counts: ResourceCounts,
}

impl OutputDrain {
    /// 吸收一次排空结果并给出判定。
    ///
    /// 溢出按超限处理：计数用 `checked_add`，绝不回绕成「仍在预算内」。
    pub fn absorb(&mut self, stream: Stream, bytes: u64, budget: &RunBudget) -> DrainVerdict {
        let (slot, max) = match stream {
            Stream::Stdout => (
                &mut self.counts.stdout_bytes,
                u64::from(budget.stdout_bytes),
            ),
            Stream::Stderr => (
                &mut self.counts.stderr_bytes,
                u64::from(budget.stderr_bytes),
            ),
        };
        match slot.checked_add(bytes) {
            Some(total) if total <= max => {
                *slot = total;
                DrainVerdict::Continue
            }
            Some(total) => {
                *slot = total;
                self.counts.output_over_budget = true;
                DrainVerdict::OverBudget
            }
            None => {
                self.counts.output_over_budget = true;
                DrainVerdict::OverBudget
            }
        }
    }

    /// 当前计数；进入审计 Outcome 的固定资源计数。
    #[must_use]
    pub fn counts(&self) -> ResourceCounts {
        self.counts
    }

    /// 是否允许解析本次输出；超限过就永远不允许。
    #[must_use]
    pub fn parse_allowed(&self) -> bool {
        !self.counts.output_over_budget
    }
}

/// 配置查询输出必须完整落在预算内才允许解析。
///
/// # 错误
/// 超过 `config_json_bytes` 返回 `BudgetUnsupported`。
pub fn config_output_acceptable(
    drain: &OutputDrain,
    budget: &RunBudget,
) -> Result<(), ContainmentGap> {
    if drain.parse_allowed() && drain.counts.stdout_bytes <= u64::from(budget.config_json_bytes) {
        Ok(())
    } else {
        Err(ContainmentGap::BudgetUnsupported)
    }
}
