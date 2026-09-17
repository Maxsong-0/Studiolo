//! 内部持久模型；公开 DTO 按职责分文件，保持单文件有界。
pub use super::outcome::*;
pub use super::public::*;
use crate::{Baseline, Binding, Digest, Input, Limits, PlanningCursor};
pub(crate) type Result<T> = std::result::Result<T, BridgeError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Progress {
    pub key: String,
    pub issued: bool,
    pub receipt: Option<EffectReceipt>,
    pub intent_ack: Option<String>,
    pub outcome_ack: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pending {
    pub input: Input,
    pub cached_plan: Box<crate::Plan>,
    pub cached_hash: Digest,
    pub actions: Vec<Progress>,
    pub summary_ack: Option<String>,
    pub index: std::collections::BTreeMap<studiolo_core::VaultPath, usize>,
    pub intent_waiting: std::collections::BTreeSet<usize>,
    pub outcome_waiting: std::collections::BTreeSet<usize>,
    pub unknown_count: usize,
    pub failed_count: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fuse {
    pub input: Input,
    pub ack: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Retirement {
    pub old_sequence: u64,
    pub old_digest: Digest,
    pub old_floor: u64,
    pub ack: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub version: u64,
    pub binding: Binding,
    pub limits: Limits,
    pub cursor: PlanningCursor,
    pub baseline: Baseline,
    pub pending: Option<Pending>,
    pub last: Option<Pending>,
    pub floor: u64,
    pub retirement: Option<Retirement>,
    pub fuse: Option<Fuse>,
}

/// 分表的更新范围；单动作不读取或重写不可变 Input/其他动作。
#[derive(Clone, Copy)]
pub(crate) enum Edit {
    Full,
    Action { sequence: u64, index: usize },
    Summary,
    Retirement,
    FuseAck,
}

impl State {
    pub fn view(&self) -> StateView {
        StateView {
            version: self.version,
            cursor: self.cursor.clone(),
            baseline: self.baseline.clone(),
            pending_sequence: self.pending.as_ref().map(|p| p.input.plan_sequence),
            last_sequence: self.last.as_ref().map(|p| p.input.plan_sequence),
            retired_through: self.floor,
            fused: self.fuse.is_some(),
        }
    }
}
