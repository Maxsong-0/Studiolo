//! Bounded attempt budgets and monotonic progress.
use serde::{Deserialize, Serialize};

/// ADR-018 work and elapsed-time thresholds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RunBudget {
    /// First pacing reminder threshold.
    pub pacing_tools: u64,
    /// Begin preparing a receipt at this threshold.
    pub wrapup_tools: u64,
    /// No additional tools after this ceiling.
    pub max_tools: u64,
    /// Begin preparing a receipt after this many seconds.
    pub wrapup_seconds: u64,
    /// Request cancellation after this many seconds.
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
/// Budget state sent to lifecycle owners.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPhase {
    /// Normal execution.
    #[default]
    Working,
    /// Pacing reminder is due.
    Pacing,
    /// Receipt preparation is due.
    Wrapup,
    /// Further tool calls must be rejected.
    Exhausted,
}
/// Monotonic observations derived from CLI protocol events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerProgress {
    /// Completed or started tool calls observed once per event identity.
    pub tool_calls: u64,
    /// File-change events observed once per event identity.
    pub edits: u64,
    /// Budget phase at observation time.
    pub phase: BudgetPhase,
}
impl RunBudget {
    /// Rejects inverted thresholds and unbounded duration.
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
    /// Computes a monotonic phase from the two enforced dimensions.
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
