//! 能力票据：绑定 owner 实例 epoch、租约、run/序号、配置摘要与代次。
//!
//! 票据只是「这次授权」的凭证，本身不提供任何 OS 能力，也不能被反序列化伪造。

use super::approval::RootPins;
use super::error::{Result, TransportError};
use super::state::TransportState;
use super::types::RunId;
use std::{num::NonZeroU64, path::Path, path::PathBuf};

/// 票据的完整绑定；私有字段，只有本 crate 能构造。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TicketBinding {
    pub epoch: [u8; 16],
    pub lease: [u8; 16],
    pub run_id: RunId,
    pub sequence: u64,
    pub config_digest: [u8; 32],
    pub config_epoch: NonZeroU64,
    pub roots: RootPins,
    pub runtime_digest: [u8; 32],
    pub issued_generation: u64,
    pub mirror_epoch: u64,
}

impl TicketBinding {
    /// 复验票据是否仍然对应同一 owner 实例、同一租约与同一运行。
    ///
    /// # Errors
    /// 跨实例/跨租约返回 `TicketForeign`；配置或根变化返回 `TicketStale`/`RootsChanged`。
    pub(crate) fn check(
        &self,
        state: &TransportState,
        epoch: [u8; 16],
        lease: [u8; 16],
        mirror_epoch: u64,
    ) -> Result<()> {
        if self.epoch != epoch || self.lease != lease {
            return Err(TransportError::TicketForeign);
        }
        if self.mirror_epoch != mirror_epoch
            || self.config_digest != state.binding.approved_config_digest
            || self.config_epoch != state.binding.scope_epoch
        {
            return Err(TransportError::TicketStale);
        }
        if self.roots != state.roots {
            return Err(TransportError::RootsChanged);
        }
        if self.runtime_digest != super::approval::runtime_digest(&state.runtime) {
            return Err(TransportError::RuntimeChanged);
        }
        if state.generation < self.issued_generation {
            return Err(TransportError::GenerationConflict);
        }
        let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
        if record.run_id != self.run_id || record.sequence != self.sequence {
            return Err(TransportError::TicketForeign);
        }
        Ok(())
    }
}

/// 已可靠发布的准备票据；私有字段，不可构造也不可反序列化。
///
/// ```compile_fail
/// let prepared: studiolo_sync_transport::owned_run::PreparedRun =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct PreparedRun {
    pub(crate) binding: TicketBinding,
}

impl PreparedRun {
    /// 本次运行标识。
    #[must_use]
    pub fn run_id(&self) -> RunId {
        self.binding.run_id
    }

    /// 本次运行序号。
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.binding.sequence
    }
}

/// 已放行一次的运行票据；借用它至多放行一次。
///
/// ```compile_fail
/// let owned: studiolo_sync_transport::owned_run::OwnedRun =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct OwnedRun {
    pub(crate) binding: TicketBinding,
}

impl OwnedRun {
    /// 本次运行标识。
    #[must_use]
    pub fn run_id(&self) -> RunId {
        self.binding.run_id
    }

    /// 本次运行序号。
    #[must_use]
    pub fn sequence(&self) -> u64 {
        self.binding.sequence
    }
}

/// 仅限本次完整本地观察的静止能力；不是远端送达回执。
///
/// 下一次 prepare、根变化、租约变化或 owner 重开都会使它失效。
///
/// ```compile_fail
/// let mirror: studiolo_sync_transport::owned_run::QuiescentMirror =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, PartialEq, Eq)]
pub struct QuiescentMirror {
    pub(crate) binding: TicketBinding,
    pub(crate) mirror_root: PathBuf,
    pub(crate) mirror_identity: (u64, u64),
}

impl QuiescentMirror {
    /// 取得静止证据的那次运行标识。
    #[must_use]
    pub fn run_id(&self) -> RunId {
        self.binding.run_id
    }

    /// 被证明静止的镜像根。
    #[must_use]
    pub fn mirror_root(&self) -> &Path {
        &self.mirror_root
    }

    /// 镜像根在取证时刻的 `(dev, ino)`。
    #[must_use]
    pub fn mirror_identity(&self) -> (u64, u64) {
        self.mirror_identity
    }

    /// 本地静止不代表远端静止，也不代表任何文件已送达。
    #[must_use]
    pub fn remote_delivery_unknown(&self) -> bool {
        true
    }
}
