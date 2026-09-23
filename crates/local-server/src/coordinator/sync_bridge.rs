//! 双向 Bridge 对 canonical 的唯一接缝（ADR-025 D8）：完整观察、有界读、条件写与条件删除。
//!
//! 本文件是 `sync_plane` 触及 Broker 的唯一位置，因此住在 `coordinator/` 之下受同一写者
//! 边界守卫；每个方法都在持有 owner 锁的情况下复验根身份，写入主体一律
//! 「外部：Obsidian」（`MutationActor::Obsidian`），幂等键由调用方按执行器原键给出。
//! 精确的 Broker 调用形态在 W1 的守卫登记里逐条列名，W3 只能用那些形态。

// B 轮 W3 接线后删除本 allow：接缝方法在驱动器调用之前没有使用者。
#![allow(dead_code)]

use crate::CanonicalWorkbench;
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    BlobIdentity, CanonicalRead, CanonicalSyncObservation, FileOperationReceipt, MutationReceipt,
    SyncReadSelection,
};

/// 接缝的固定失败分类；不带路径或正文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeSeamError {
    /// 写者未就绪、关闭中或根身份漂移。
    Unavailable,
    /// 观察不完整（诊断非空）或范围外路径。
    Incomplete,
    /// CAS 前置不满足；权威拒绝，可记 NoEffect。
    Rejected,
    /// 已发出安装但终态无法确认。
    Unknown,
    /// 审计账本不可用。
    Audit,
}

/// 条件写的目标前置：不存在（创建）或等于某 revision（替换）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgePrecondition {
    /// 目标必须不存在。
    MustNotExist,
    /// 目标当前必须是此 revision。
    Match(Revision),
}

impl CanonicalWorkbench {
    /// 完整观察批准范围（两阶段：元数据调查 → 增补 `required` → 逐文件流式事实）。
    ///
    /// # Errors
    /// 写者不可用返回 `Unavailable`；范围外或不完整返回 `Incomplete`。
    pub(crate) fn bridge_observe(
        &self,
        selection: &SyncReadSelection,
        required: &[VaultPath],
    ) -> Result<CanonicalSyncObservation, BridgeSeamError> {
        // B 轮 W3 实现：owner 锁内 begin_sync_observation → finish_sync_observation。
        let _ = (selection, required);
        Err(BridgeSeamError::Unavailable)
    }

    /// 有界读取单个 canonical 文件（≤ `max_bytes`）；用于正文搬运与保全。
    ///
    /// # Errors
    /// 不存在或不稳定返回 `Rejected`；写者不可用返回 `Unavailable`。
    pub(crate) fn bridge_read(
        &self,
        path: &VaultPath,
        max_bytes: u64,
    ) -> Result<CanonicalRead, BridgeSeamError> {
        self.read_file(path, max_bytes)
            .map_err(|_| BridgeSeamError::Unavailable)
    }

    /// 整篇条件写（≤ 4 MiB 正文）：创建或按 revision 替换，主体「外部：Obsidian」。
    ///
    /// # Errors
    /// CAS 拒绝返回 `Rejected`；安装后无法确认返回 `Unknown`。
    pub(crate) fn bridge_write(
        &self,
        path: &VaultPath,
        content: Vec<u8>,
        precondition: BridgePrecondition,
        idempotency_key: &str,
    ) -> Result<MutationReceipt, BridgeSeamError> {
        // B 轮 W3 实现：先原键重放查询，再条件写；主体 Obsidian、种类 Replace。
        let _ = (path, content, precondition, idempotency_key);
        Err(BridgeSeamError::Unavailable)
    }

    /// 大对象条件写（> 4 MiB，≤ 256 MiB）：正文来自外 stage 的只读来源，走流式准备／提交。
    ///
    /// # Errors
    /// 同 `bridge_write`。
    pub(crate) fn bridge_write_stream(
        &self,
        path: &VaultPath,
        stage_root: &std::path::Path,
        stage_object: &VaultPath,
        content: BlobIdentity,
        precondition: BridgePrecondition,
        idempotency_key: &str,
    ) -> Result<MutationReceipt, BridgeSeamError> {
        // B 轮 W3 实现：以 stage_root 与 stage_object 建只读附件来源 → 流式准备 → Intent 审计 → 流式提交。
        let _ = (
            path,
            stage_root,
            stage_object,
            content,
            precondition,
            idempotency_key,
        );
        Err(BridgeSeamError::Unavailable)
    }

    /// 条件删除：走文件操作协议（保全进墓碑，可恢复），主体「外部：Obsidian」。
    ///
    /// # Errors
    /// CAS 拒绝返回 `Rejected`；审计不可用返回 `Audit`。
    pub(crate) fn bridge_delete(
        &self,
        path: &VaultPath,
        expected: Revision,
        idempotency_key: &str,
        scope_hash: &str,
        scope_epoch: u64,
    ) -> Result<FileOperationReceipt, BridgeSeamError> {
        // B 轮 W3 实现：prepare_file_operation → Intent 审计 → commit_file_operation。
        let _ = (path, expected, idempotency_key, scope_hash, scope_epoch);
        Err(BridgeSeamError::Unavailable)
    }

    /// 冲刷内容与文件操作两条审计 outbox；任一失败返回 `Audit`。
    ///
    /// # Errors
    /// 审计不可用返回 `Audit`。
    pub(crate) fn bridge_flush_audit(&self) -> Result<(), BridgeSeamError> {
        // B 轮 W3 实现：flush_content_audit + flush_file_operation_audit。
        Err(BridgeSeamError::Audit)
    }
}
