//! 受信 owner 的窄协调入口；审阅票据不是授权，协调不能制造原请求成功。

mod evidence;
pub(crate) mod maintenance;
mod observation;
mod proposed;
mod prune;
mod run;
mod terminal;
#[cfg(test)]
#[path = "../tests/recovery_private/mod.rs"]
mod tests;

use crate::{Revision, VaultPath};
use std::sync::Arc;

/// 写者日志的阶段观测；Prepared 只允许读取，仍禁止另一笔写入。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterPhase {
    /// 没有在途操作。
    Ready,
    /// 尚未尝试安装，可取消准备。
    Prepared,
    /// 安装结果未知，不猜测成功。
    Installing,
    /// 已有完成日志，待补真回执与水位。
    Completed,
    /// 旧格式未知请求没有足够协调证据。
    LegacyUnknown,
    /// 本实例持久化结果未知，须先重新打开。
    Uncertain,
}

/// 一次性审阅票据：不可构造、反序列化或跨实例使用，不保活写者租约。
pub struct PendingMutation {
    instance: Arc<()>,
    ticket: u64,
    state: crate::mutation::State,
    operation: crate::mutation::record::Operation,
    vault_uuid: crate::VaultUuid,
    journal: observation::Observed,
    current: observation::Observed,
    stage: observation::Observed,
}

impl PendingMutation {
    /// 原请求的逻辑路径。
    pub fn path(&self) -> &VaultPath {
        &self.operation().path
    }
    /// 原请求的固定操作序号，不代表已提交。
    pub fn sequence(&self) -> u64 {
        self.operation().sequence
    }
    /// 原请求阶段。
    pub fn phase(&self) -> WriterPhase {
        match self.operation().phase {
            crate::mutation::record::Phase::Prepared => WriterPhase::Prepared,
            _ => WriterPhase::Installing,
        }
    }
    /// 原请求要求的旧 revision。
    pub fn previous_revision(&self) -> Option<Revision> {
        self.operation().previous_revision
    }
    /// 原请求拟安装的 revision。
    pub fn target_revision(&self) -> Revision {
        self.operation().target_revision
    }
    /// 审阅时实际目标 revision，缺失为 None。
    pub fn current_revision(&self) -> Option<Revision> {
        self.current.revision.or_else(|| {
            self.current
                .bytes
                .as_ref()
                .map(|bytes| Revision::compute(self.vault_uuid, self.path(), bytes))
        })
    }
    fn operation(&self) -> &crate::mutation::record::Operation {
        &self.operation
    }
}

/// 主人确认后的封闭决策；调用方负责认证、近期因子和审计。
pub enum PendingDecision {
    /// 只取消尚未安装的准备态；不会把取消伪装成 CAS 拒绝。
    CancelPrepared,
    /// 保留当前 canonical，封存双方；原内容须来自可信原请求或离线输入。
    PreserveCurrent {
        /// 经目标 revision、摘要和长度核对的原请求正文，上限 4 MiB。
        original_content: Vec<u8>,
    },
}

/// 协调结果，始终不是原 mutation 的成功回执。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReconciliationReceipt {
    /// 被协调路径。
    pub path: VaultPath,
    /// 原请求序号。
    pub sequence: u64,
    /// 取消或未知结果。
    pub outcome: ReconciliationOutcome,
    /// 私有证据 ID；取消准备态不需要证据包。
    pub evidence_id: Option<String>,
    /// 决策时观察的目标，不能代表将来的页面状态。
    pub current_revision: Option<Revision>,
    /// 原有提交水位；协调不把未知安装提升为提交。
    pub committed_through: u64,
}

/// 非成功终态的两种原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    /// 尚未安装即取消。
    Cancelled,
    /// 原安装结果永久未知，保留当前并封存证据。
    Indeterminate,
}

/// 自有无归属 stage 的一次有界整理结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StagePruneReport {
    /// 完整验证后实际删除的数量。
    pub removed: usize,
    /// 当前 Prepared 操作仍拥有的数量。
    pub retained: usize,
}

pub(crate) use terminal::finish;
