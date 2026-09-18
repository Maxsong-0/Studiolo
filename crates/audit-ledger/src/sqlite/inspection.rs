//! 分段账本的显式运维入口：一次性升级与按段、全量的独立验证。
//!
//! 这些入口都不在 append/query 的热路径上：它们的成本可以与账本总量成正比，
//! 因此必须由部署方显式调度，本 Adapter 不自行触发，也不把它们的结果缓存成「已验证」。

use super::{
    check_configuration, connect, schema, storage::CreatePolicy, storage::Storage, unavailable,
    upgrade, verify,
};
use crate::{AuditError, AuditSegmentSummary, AuditVerificationCost, SqliteAuditLedger};
use rusqlite::TransactionBehavior;
use std::path::Path;

impl SqliteAuditLedger {
    /// 显式把 T-012 的 v1 账本升级为分段格式，返回本次全链验证的规模。
    ///
    /// 只接受恰好是 v1 的既有库：v2、未知版本、结构或链校验不过一律拒绝并回滚，
    /// 不隐式补表也不修补。升级不改写任何既有记录，段摘要全部由已验证的记录链重算得出；
    /// 升级过程成本与账本总量成正比，是一次性的人工运维动作，不在 open 路径上自动发生。
    pub fn upgrade_existing(private_directory: &Path) -> Result<AuditVerificationCost, AuditError> {
        let storage = Storage::prepare(private_directory, CreatePolicy::ExistingOnly)?;
        let mut connection = connect(&storage)?;
        check_configuration(&connection)?;
        let mode: String = connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(unavailable)?;
        if mode != "wal" {
            return Err(AuditError::Unavailable);
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        let cost = upgrade::run(&transaction)?;
        storage.check()?;
        transaction.commit().map_err(unavailable)?;
        storage.check()?;
        Ok(cost)
    }

    /// 全量验证：物理结构、整表行数与 offset 边界、段链与每一段的记录链。
    ///
    /// 这是本 Adapter 对「账本完整」的完整定义，成本与账本总量成正比。常规 append/query
    /// 只验证段链、打开段与读到的段，因此**已封存段内部的改写要靠本方法或读到该段的查询
    /// 才会发现**；把它编排成定期巡检是部署方的责任，本 Adapter 不自行调度。
    pub fn verify_all(&self) -> Result<AuditVerificationCost, AuditError> {
        self.storage.check()?;
        let mut session = self.lock()?;
        let transaction = session
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        let cost = verify::all(&transaction)?;
        self.storage.check()?;
        transaction.commit().map_err(unavailable)?;
        session.cost = cost;
        Ok(cost)
    }

    /// 验证单独一段的记录链，并把它锚回整条段链；段号超出已封存范围返回 `Ok(None)`。
    ///
    /// 用于中断后按段续验：调用方可以在重启后从任意段号继续，而不必从头重扫整个账本。
    pub fn verify_segment(
        &self,
        segment_index: u64,
    ) -> Result<Option<AuditSegmentSummary>, AuditError> {
        self.storage.check()?;
        let mut session = self.lock()?;
        let transaction = session
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        schema::validate(&transaction, schema::VERSION)?;
        let anchored = verify::anchor(&transaction)?;
        let mut cost = anchored.cost;
        // 段号超出已封存范围（包括在 32 位平台上超出 usize）不是损坏，只是没有这一段。
        let summary = match usize::try_from(segment_index)
            .ok()
            .and_then(|position| anchored.summaries.get(position))
        {
            None => None,
            Some(summary) => {
                let summary = *summary;
                cost.merge(verify::segment_records(
                    &transaction,
                    &anchored,
                    segment_index,
                    &mut |_| Ok(()),
                )?);
                Some(summary)
            }
        };
        self.storage.check()?;
        transaction.commit().map_err(unavailable)?;
        session.cost = cost;
        Ok(summary)
    }

    /// 返回已验证的全部封存段摘要；只重算段链，不走读段内记录。
    pub fn segments(&self) -> Result<Vec<AuditSegmentSummary>, AuditError> {
        self.storage.check()?;
        let mut session = self.lock()?;
        let transaction = session
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(unavailable)?;
        schema::validate(&transaction, schema::VERSION)?;
        let anchored = verify::anchor(&transaction)?;
        self.storage.check()?;
        transaction.commit().map_err(unavailable)?;
        session.cost = anchored.cost;
        Ok(anchored.summaries)
    }

    /// 返回最近一次成功操作实际读入并重算的规模；用于核对分段成本，不是完整性结论。
    pub fn last_verification_cost(&self) -> Result<AuditVerificationCost, AuditError> {
        Ok(self.lock()?.cost)
    }
}
