//! 静止依赖：执行器只接受受控镜像 owner 给出的静止证据，自己不推断、不降级。
//!
//! 证据语义照 `docs/m2/owned-run-design.md`：只认「同一受控 payload 子组已 populated=0
//! 且继续 spawn 入口已关闭」。`MainPID=0` / `ActiveState=inactive` / `Child.wait` / PGID /
//! pipe EOF 都不是静止，必须由实现方返回 [`QuiescenceEvidence::Unknown`]。

use crate::{Digest, Identity};
use std::time::Instant;

/// 本批执行对静止窗口的精确请求；不是授权，也不能放宽实现方的判据。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuiescenceRequest {
    /// 本次同步链。
    pub link_id: Identity,
    /// 规则代次。
    pub scope_epoch: u64,
    /// 经可信装配核对的镜像根身份。
    pub mirror_root_token: Digest,
    /// 静止证据的镜像观察序号不得早于本批冻结观察。
    pub minimum_mirror_observation: u64,
    /// 本批允许的最晚有效时刻；证据窗口更长时按此截断。
    pub not_after: Instant,
}

/// 受控镜像给出的静止判据闭集；只有一个变体代表真的静止。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuiescenceEvidence {
    /// 同一受控 payload 子组 `cgroup.events populated=0`；还须已关闭继续 spawn 入口。
    ControlledSubgroupIdle {
        /// 继续 spawn 入口是否已关闭；false 一律拒绝。
        spawn_gate_closed: bool,
    },
    /// 平台前置未满足（macOS、无权限 Linux）；绝不降级为静止。
    Unsupported,
    /// 无法判定；绝不降级为静止。
    Unknown,
}

/// 受控镜像 owner 返回的原始证据；执行器只核验，不构造。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirrorQuiescenceProof {
    /// 证据所属同步链。
    pub link_id: Identity,
    /// 证据所属规则代次。
    pub scope_epoch: u64,
    /// 证据所属镜像根身份。
    pub mirror_root_token: Digest,
    /// 实际判据。
    pub evidence: QuiescenceEvidence,
    /// 该静止窗口对应的镜像观察序号。
    pub mirror_observation: u64,
    /// 单调时钟截止点；过期即失效。
    pub expires_at: Instant,
}

/// 取不到静止的闭集原因；每一项都表示「不搬运」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuiescenceFault {
    /// 受控镜像 owner 当前不可用。
    Unavailable,
    /// 平台前置未满足。
    Unsupported,
    /// 判据未知。
    Unknown,
    /// 子组空闲但继续 spawn 入口仍开着。
    SpawnGateOpen,
    /// 证据绑定的链路、代次或镜像根与本批不符。
    BindingMismatch,
    /// 证据窗口已过期。
    Expired,
    /// 证据的镜像观察早于本批冻结观察。
    Stale,
}

/// 受控镜像 owner 能力；B3 的 `QuiescentMirror` 落地后由适配层实现本 trait。
pub trait MirrorQuiescence {
    /// 返回当前静止证据；不可用/未知/不支持必须显式失败，不得返回伪静止。
    fn observe(
        &self,
        request: &QuiescenceRequest,
    ) -> Result<MirrorQuiescenceProof, QuiescenceFault>;
}

/// 核验通过的静止票据；只能由本模块铸出，外部无法构造。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quiesced {
    mirror_observation: u64,
    deadline: Instant,
}

impl Quiesced {
    /// 静止窗口对应的镜像观察序号。
    pub fn mirror_observation(&self) -> u64 {
        self.mirror_observation
    }
    /// 本批实际可用的最晚时刻：证据窗口与请求窗口取较早者。
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// 逐项核验证据；任何一项不符都返回失败，绝不补造静止。
pub(crate) fn admit(
    request: &QuiescenceRequest,
    proof: &MirrorQuiescenceProof,
    now: Instant,
) -> Result<Quiesced, QuiescenceFault> {
    if proof.link_id != request.link_id
        || proof.scope_epoch != request.scope_epoch
        || proof.mirror_root_token != request.mirror_root_token
    {
        return Err(QuiescenceFault::BindingMismatch);
    }
    match proof.evidence {
        QuiescenceEvidence::ControlledSubgroupIdle {
            spawn_gate_closed: true,
        } => {}
        QuiescenceEvidence::ControlledSubgroupIdle {
            spawn_gate_closed: false,
        } => return Err(QuiescenceFault::SpawnGateOpen),
        QuiescenceEvidence::Unsupported => return Err(QuiescenceFault::Unsupported),
        QuiescenceEvidence::Unknown => return Err(QuiescenceFault::Unknown),
    }
    if proof.mirror_observation < request.minimum_mirror_observation {
        return Err(QuiescenceFault::Stale);
    }
    let deadline = proof.expires_at.min(request.not_after);
    if now >= deadline {
        return Err(QuiescenceFault::Expired);
    }
    Ok(Quiesced {
        mirror_observation: proof.mirror_observation,
        deadline,
    })
}
