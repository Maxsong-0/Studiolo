//! 已 Issued 动作的实际执行者：canonical 侧走 `coordinator::sync_bridge` 接缝的条件写，
//! 镜像侧走 `mirror::install` 在静止窗口内原子进入视图。
//!
//! 终态只认权威回执：Broker 返回的真实 revision、镜像 renameat 后复验的身份；
//! 拿不到就是 `JobOutcome::Unknown`，绝不用退出码或「内容碰巧相同」补成成功。

use super::driver::content::{
    MirrorWriteError, RoundContext, preserve_canonical, write_from_mirror,
};
use super::driver::evidence::{hex, revision_digest, revision_from_digest, sha256};
use super::mirror::stage::StageStore;
use super::mirror::{self, StageSink};
use super::{PlaneError, naming, quiescence};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use std::cell::RefCell;
use studiolo_core::Revision;
use studiolo_sync_bridge::executor::{
    IssuedJob, JobBackend, JobOutcome, MirrorObjectSink, NoEffectCause, ObjectId, TransferFault,
};
use studiolo_sync_bridge::{Action, Binding, Change, FileVersion, Side};
use studiolo_sync_transport::owned_run::SyncMode;
use studiolo_vault_broker::FileOperationOutcome;

/// 一批执行期间的后端；借用本轮上下文（静止票据、视图、外 stage）。
pub struct PlaneJobBackend<'a> {
    context: &'a RoundContext<'a>,
    binding: Binding,
}

impl<'a> PlaneJobBackend<'a> {
    /// 组装一批的执行后端。
    #[must_use]
    pub fn new(context: &'a RoundContext<'a>, binding: Binding) -> Self {
        Self { context, binding }
    }

    /// 冻结版本对应的原 `Revision`：Bridge 证据里的 `revision` 就是它的 32 字节原值，任何轮都能还原。
    fn revision(version: &FileVersion) -> Revision {
        revision_from_digest(&version.revision)
    }

    /// canonical 侧按原键重建首次请求走接缝：CreateOnly／Replace 走条件写（前置＝冻结 `target_expected`
    /// 还原的 revision），Delete 走文件操作删除；主体一律「外部：Obsidian」。首次执行与重开核对共用
    /// 这一份，保证核对时的请求与首次逐字段相同（接缝先按原键查重放，有原回执绝不第二次安装）。
    pub(crate) fn canonical_verdict(&self, job: &IssuedJob, action: &Action) -> SeamVerdict {
        let workbench = self.context.plane.workbench();
        let path = &action.key.path;
        match &action.change {
            Change::Delete { target_expected } => {
                let expected = Self::revision(target_expected);
                let deleted = workbench.bridge_delete(
                    path,
                    expected,
                    &job.external_key,
                    &hex(&self.binding.scope_hash),
                    self.binding.scope_epoch,
                );
                match deleted {
                    Ok(receipt) => SeamVerdict::Outcome(match receipt.outcome {
                        FileOperationOutcome::Committed => JobOutcome::Committed {
                            new_revision: None,
                            content_hash: None,
                            bytes: None,
                            receipt_id: receipt.operation_id,
                        },
                        FileOperationOutcome::Rejected => JobOutcome::NoEffect {
                            reason: NoEffectCause::CasRejected,
                            terminal_id: receipt.operation_id,
                        },
                        FileOperationOutcome::Cancelled => JobOutcome::NoEffect {
                            reason: NoEffectCause::CancelledBeforeInstall,
                            terminal_id: receipt.operation_id,
                        },
                        FileOperationOutcome::Indeterminate => JobOutcome::Unknown,
                    }),
                    Err(error) => seam_verdict(error),
                }
            }
            Change::CreateOnly { source } | Change::Replace { source, .. } => {
                let precondition = match &action.change {
                    Change::Replace {
                        target_expected, ..
                    } => BridgePrecondition::Match(Self::revision(target_expected)),
                    _ => BridgePrecondition::MustNotExist,
                };
                let written =
                    write_from_mirror(self.context, path, source, precondition, &job.external_key);
                match written {
                    Ok(receipt) => SeamVerdict::Outcome(JobOutcome::Committed {
                        new_revision: Some(revision_digest(receipt.revision)),
                        content_hash: Some(source.sha256),
                        bytes: Some(source.bytes),
                        receipt_id: receipt.revision.to_hex(),
                    }),
                    Err(MirrorWriteError::Source) => SeamVerdict::SourceUnavailable,
                    Err(MirrorWriteError::Seam(error)) => seam_verdict(error),
                }
            }
        }
    }

    /// 镜像侧：每次先 `verify_quiescent`；MirrorRemote 下出站前把 canonical 当前版本保全到
    /// `ops/bridge-rollback/`（ADR-025 D5）；再把 stage 对象 renameat 进视图。
    fn apply_mirror(
        &mut self,
        job: &IssuedJob,
        action: &Action,
        staged: Option<ObjectId>,
    ) -> Result<JobOutcome, PlaneError> {
        quiescence::verify(self.context.plane, self.context.ticket)?;
        let path = &action.key.path;
        let view = &self.context.view;
        match &action.change {
            Change::CreateOnly { source } | Change::Replace { source, .. } => {
                // 出站动作的源就是 canonical 当前版本；引导期它会被下一次 RunOnce 回滚，先保全。
                // 保全失败时视图还没被碰：本动作确定未安装，明确无效果，下一轮重新规划。
                // 回滚保全只在首次执行里发生一次（重开核对不重放它），原键配本轮分钟戳即可。
                let stamp = naming::utc_minute_stamp(self.context.now_unix);
                if self.context.mode == SyncMode::MirrorRemote
                    && stamp.is_none_or(|stamp| {
                        let key = format!("{}/rollback", job.external_key);
                        preserve_canonical(
                            self.context,
                            naming::ROLLBACK_DIR,
                            path,
                            source,
                            &stamp,
                            &key,
                        )
                        .is_err()
                    })
                {
                    return Ok(not_attempted(&job.external_key));
                }
                let object = staged.ok_or(PlaneError::Storage)?;
                let target_expected = match &action.change {
                    Change::Replace {
                        target_expected, ..
                    } => Some(target_expected),
                    _ => None,
                };
                let installed = mirror::install(
                    view,
                    &mut self.context.stage.borrow_mut(),
                    object,
                    path,
                    target_expected,
                );
                Ok(match installed {
                    Ok(version) => JobOutcome::Committed {
                        new_revision: Some(version.revision),
                        content_hash: Some(version.sha256),
                        bytes: Some(version.bytes),
                        receipt_id: hex(&version.revision),
                    },
                    Err(PlaneError::Observation) => rejected(&job.external_key),
                    Err(_) => JobOutcome::Unknown,
                })
            }
            Change::Delete { target_expected } => {
                let removed = mirror::remove(
                    view,
                    &mut self.context.stage.borrow_mut(),
                    path,
                    target_expected,
                    job.key.plan.sequence,
                    self.context.now_unix,
                );
                Ok(match removed {
                    Ok(object) => JobOutcome::Committed {
                        new_revision: None,
                        content_hash: None,
                        bytes: None,
                        receipt_id: hex(&object.digest()),
                    },
                    Err(PlaneError::Observation) => rejected(&job.external_key),
                    Err(_) => JobOutcome::Unknown,
                })
            }
        }
    }
}

/// 接缝一次条件写／删除的结论（公开投影，契约测试可直接喂）；首次执行与重开核对各自映射成终态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeamVerdict {
    /// 后端按原键给出的回执（提交、经确认的无效果，或回执本身声明的未定）。
    Outcome(JobOutcome),
    /// 还没碰 Broker 就失败：本次调用确定没有安装。
    NotAttempted,
    /// 镜像源读不出或已不是冻结版本：请求没发出去。首次执行等同 `NotAttempted`；重开核对时
    /// 原请求重建不出、原键下可能早已安装，只能保持未知。
    SourceUnavailable,
    /// 权威 CAS 拒绝。
    Rejected,
    /// 终态无法确认。
    Unknown,
}

/// 接缝失败分类到结论：`Unavailable`／`Audit`／`Unknown` 一律未知。
pub(crate) fn seam_verdict(error: BridgeSeamError) -> SeamVerdict {
    match error {
        BridgeSeamError::NotAttempted => SeamVerdict::NotAttempted,
        BridgeSeamError::Rejected => SeamVerdict::Rejected,
        _ => SeamVerdict::Unknown,
    }
}

/// 首次执行的映射：未碰 Broker 即明确无效果（计划终结、基线不前进、下一轮重新规划），CAS 拒绝记无效果，其余未知。
#[must_use]
pub fn first_attempt_outcome(verdict: SeamVerdict, external_key: &str) -> JobOutcome {
    match verdict {
        SeamVerdict::Outcome(outcome) => outcome,
        SeamVerdict::NotAttempted | SeamVerdict::SourceUnavailable => not_attempted(external_key),
        SeamVerdict::Rejected => rejected(external_key),
        SeamVerdict::Unknown => JobOutcome::Unknown,
    }
}

/// 未碰后端的固定无效果回执：动作确定未安装；终态标识由原键派生，不带路径。
#[must_use]
pub fn not_attempted(external_key: &str) -> JobOutcome {
    JobOutcome::NoEffect {
        reason: NoEffectCause::CancelledBeforeInstall,
        terminal_id: format!("not-attempted:{external_key}"),
    }
}

/// 权威 CAS 拒绝的固定无效果回执；终态标识由原键派生，不带路径。
#[must_use]
pub fn rejected(external_key: &str) -> JobOutcome {
    JobOutcome::NoEffect {
        reason: NoEffectCause::CasRejected,
        terminal_id: format!("cas-rejected:{}", hex(&sha256(external_key.as_bytes()))),
    }
}

impl JobBackend for PlaneJobBackend<'_> {
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome {
        match action.key.target {
            Side::Canonical => {
                first_attempt_outcome(self.canonical_verdict(job, action), &job.external_key)
            }
            // 镜像侧的平面失败只能是未知：执行器不重发，下一轮按原键核对。
            Side::Mirror => self
                .apply_mirror(job, action, staged)
                .unwrap_or(JobOutcome::Unknown),
        }
    }
}

/// 让外 stage 同时服务执行器的落库出口与本后端的安装：每次调用短暂借用，
/// 具体形态仍是 `mirror::StageSink`，这里不复制它的任何逻辑。
pub struct SharedStageSink<'a> {
    stage: &'a RefCell<StageStore>,
}

impl<'a> SharedStageSink<'a> {
    /// 绑定到本轮打开的外 stage。
    #[must_use]
    pub fn new(stage: &'a RefCell<StageStore>) -> Self {
        Self { stage }
    }
}

impl MirrorObjectSink for SharedStageSink<'_> {
    fn begin(&mut self) -> Result<(), TransferFault> {
        StageSink::new(&mut self.stage.borrow_mut()).begin()
    }

    fn accept(&mut self, frame: &[u8]) -> Result<(), TransferFault> {
        StageSink::new(&mut self.stage.borrow_mut()).accept(frame)
    }

    fn finish(&mut self, id: ObjectId) -> Result<(), TransferFault> {
        StageSink::new(&mut self.stage.borrow_mut()).finish(id)
    }

    fn discard(&mut self) -> Result<(), TransferFault> {
        StageSink::new(&mut self.stage.borrow_mut()).discard()
    }
}
