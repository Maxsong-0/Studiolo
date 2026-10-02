//! 整批执行顺序：先算整批 → 取静止 → 逐动作 → 收尾审计 → 结果闭合。
//!
//! 顺序本身就是契约：任何一步失败都在发出第一条删除之前返回，
//! 未知终态永远不会被折叠成成功，也不会自动推进共同基线。

mod guards;
mod jobs;

use super::{
    cas::{MirrorCas, ObjectId, StageUsage},
    deletion::{self, DeletionAudit},
    error::ExecutorError,
    journal::{ExecutionWitness, PlanJournal},
    quiescence::{MirrorQuiescence, Quiesced},
    transfer::{JobBackend, MirrorContent, MirrorObjectSink},
};
use crate::Plan;
use std::time::Instant;

/// 本次执行需要的全部外部能力；执行器自己不持有其中任何一项。
pub struct ExecutionPorts<'a> {
    /// 受控镜像静止能力。
    pub quiescence: &'a dyn MirrorQuiescence,
    /// 持久状态 owner。
    pub journal: &'a mut dyn PlanJournal,
    /// 可信观察 adapter。
    pub witness: &'a mut dyn ExecutionWitness,
    /// 冻结版本的正文来源。
    pub content: &'a mut dyn MirrorContent,
    /// 受控镜像内容寻址落库出口。
    pub sink: &'a mut dyn MirrorObjectSink,
    /// 已 Issued 动作的实际执行者。
    pub backend: &'a mut dyn JobBackend,
    /// 本次使用的 MirrorCAS 账本。
    pub cas: &'a mut MirrorCas,
}

/// 整批执行结果；每个计数都只反映已被证据支持的事实。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionReport {
    /// 本批计划序号。
    pub sequence: u64,
    /// 整批删除复核结论。
    pub deletion: DeletionAudit,
    /// 采信的静止窗口对应镜像观察序号。
    pub quiescent_mirror_observation: u64,
    /// 本次新持久化 Issued 的动作数。
    pub issued: usize,
    /// 已取得提交回执的动作数。
    pub committed: usize,
    /// 已取得明确无效果回执的动作数。
    pub no_effect: usize,
    /// 终态仍未知的动作数；未知不是失败也不是成功。
    pub unknown: usize,
    /// 进入本次执行前就已有持久状态、本次未重放的动作数。
    pub already_known: usize,
    /// 删除前已保全、须由镜像 owner 保留的对象；本片不承诺保留期限。
    pub preserved: Vec<ObjectId>,
    /// 本批冲突数；执行器不碰冲突路径，也不冒称冲突字节已保存。
    pub conflicts: usize,
    /// 是否已闭合本计划。
    pub settled: bool,
    /// 共同基线版本是否实际前进（由持久层前后快照比较得出）。
    pub baseline_advanced: bool,
    /// 收尾时的 MirrorCAS 占用。
    pub stage: StageUsage,
}

/// 执行一个已被持久层预留的冻结计划。
///
/// `now` 是调用方的单调时钟读数，`not_after` 是本批允许的最晚时刻。
pub fn execute(
    plan: &Plan,
    mut ports: ExecutionPorts<'_>,
    now: Instant,
    not_after: Instant,
) -> Result<ExecutionReport, ExecutorError> {
    jobs::check_stopping(ports.backend)?;
    let before = ports.journal.snapshot().map_err(ExecutorError::Journal)?;
    guards::alignment(plan, &before)?;
    // 先算整批：门槛未过则在任何动作发出、任何证据放行之前整批阻断。
    let (audit, clearance) = deletion::clear(plan)?;
    guards::evidence(plan)?;
    let quiesced = guards::quiesce(plan, ports.quiescence, audit, now, not_after)?;
    // Intent 的真实审计 append 必须先于任何 Issued。
    jobs::drain_audit(ports.journal, ports.backend)?;
    let snapshot = ports.journal.snapshot().map_err(ExecutorError::Journal)?;
    guards::alignment(plan, &snapshot)?;
    let mut report = ExecutionReport {
        sequence: plan.key().sequence,
        deletion: audit,
        quiescent_mirror_observation: quiesced.mirror_observation(),
        issued: 0,
        committed: 0,
        no_effect: 0,
        unknown: 0,
        already_known: 0,
        preserved: Vec::new(),
        conflicts: plan.conflicts().len(),
        settled: false,
        baseline_advanced: false,
        stage: StageUsage::default(),
    };
    for (index, action) in plan.actions().iter().enumerate() {
        jobs::check_stopping(ports.backend)?;
        let record = snapshot
            .jobs
            .get(index)
            .ok_or(ExecutorError::PlanMismatch)?
            .clone();
        jobs::run(
            &mut ports,
            action,
            &record,
            &quiesced,
            &clearance,
            &mut report,
        )?;
    }
    // Outcome 与整批 Summary 的真实审计 append 在结果闭合之前完成。
    jobs::drain_audit(ports.journal, ports.backend)?;
    jobs::check_stopping(ports.backend)?;
    finish(plan, &mut ports, &quiesced, &before, &mut report)?;
    report.stage = ports.cas.usage();
    Ok(report)
}

fn finish(
    plan: &Plan,
    ports: &mut ExecutionPorts<'_>,
    quiesced: &Quiesced,
    before: &super::journal::JournalSnapshot,
    report: &mut ExecutionReport,
) -> Result<(), ExecutorError> {
    if report.unknown > 0 {
        // 有未知终态就不闭合；重开后仍按原键核对，不补造成功。
        return Ok(());
    }
    // 失败、部分成功或仍有冲突时只终结计划，保留旧共同基线。
    let common = if report.no_effect == 0 && report.conflicts == 0 {
        ports
            .witness
            .common(quiesced)
            .map_err(ExecutorError::Witness)?
    } else {
        None
    };
    let current = ports.journal.snapshot().map_err(ExecutorError::Journal)?;
    let sequence = plan.key().sequence;
    match ports
        .journal
        .settle(current.version, sequence, common.clone())
    {
        // 上一份完整终态还没有清理票据：取票、拿到真实审计 ACK，再闭合本计划。
        Err(super::journal::JournalFault::RetentionRequired) => {
            let version = ports
                .journal
                .snapshot()
                .map_err(ExecutorError::Journal)?
                .version;
            ports
                .journal
                .prepare_retirement(version)
                .map_err(ExecutorError::Journal)?;
            jobs::drain_audit(ports.journal, ports.backend)?;
            let version = ports
                .journal
                .snapshot()
                .map_err(ExecutorError::Journal)?
                .version;
            ports
                .journal
                .settle(version, sequence, common)
                .map_err(ExecutorError::Journal)?;
        }
        other => other.map_err(ExecutorError::Journal)?,
    }
    let after = ports.journal.snapshot().map_err(ExecutorError::Journal)?;
    report.settled = true;
    report.baseline_advanced = after.baseline_version > before.baseline_version;
    Ok(())
}
