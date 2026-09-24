//! 已 Issued 动作的实际执行者：canonical 侧走 `coordinator::sync_bridge` 接缝的条件写，
//! 镜像侧走 `mirror::install` 在静止窗口内原子进入视图。
//!
//! 终态只认权威回执：Broker 返回的真实 revision、镜像 renameat 后复验的身份；
//! 拿不到就是 `JobOutcome::Unknown`，绝不用退出码或「内容碰巧相同」补成成功。

use super::driver::content::{RoundContext, preserve_canonical, write_from_mirror};
use super::driver::evidence::{hex, sha256};
use super::mirror::stage::StageStore;
use super::mirror::{self, StageSink};
use super::{PlaneError, naming};
use crate::coordinator::sync_bridge::{BridgePrecondition, BridgeSeamError};
use std::cell::RefCell;
use studiolo_core::Revision;
use studiolo_sync_bridge::executor::{
    IssuedJob, JobBackend, JobOutcome, MirrorObjectSink, NoEffectCause, ObjectId, TransferFault,
};
use studiolo_sync_bridge::{Action, Binding, Change, FileVersion, Side};
use studiolo_sync_transport::owned_run::{QuiescentMirror, SyncMode};
use studiolo_vault_broker::FileOperationOutcome;

/// 一批执行期间的后端；借用本轮上下文（视图、外 stage、`Revision` 表）与静止票据。
pub struct PlaneJobBackend<'a> {
    context: &'a RoundContext<'a>,
    ticket: &'a QuiescentMirror,
    binding: Binding,
}

impl<'a> PlaneJobBackend<'a> {
    /// 组装一批的执行后端。
    #[must_use]
    pub fn new(
        context: &'a RoundContext<'a>,
        ticket: &'a QuiescentMirror,
        binding: Binding,
    ) -> Self {
        Self {
            context,
            ticket,
            binding,
        }
    }

    /// 从本轮观察表还原冻结版本对应的原 `Revision`；没观察过就拿不到权威前置。
    fn revision(&self, version: &FileVersion) -> Result<Revision, PlaneError> {
        self.context
            .revisions
            .borrow()
            .restore(&version.revision)
            .ok_or(PlaneError::Observation)
    }

    /// canonical 侧：CreateOnly／Replace 走条件写（前置＝冻结 `target_expected` 还原的 revision），
    /// Delete 走文件操作删除；主体一律「外部：Obsidian」。
    fn apply_canonical(
        &mut self,
        job: &IssuedJob,
        action: &Action,
    ) -> Result<JobOutcome, PlaneError> {
        let workbench = self.context.plane.workbench();
        let path = &action.key.path;
        match &action.change {
            Change::Delete { target_expected } => {
                let expected = self.revision(target_expected)?;
                let receipt = match workbench.bridge_delete(
                    path,
                    expected,
                    &job.external_key,
                    &hex(&self.binding.scope_hash),
                    self.binding.scope_epoch,
                ) {
                    Ok(receipt) => receipt,
                    Err(BridgeSeamError::Rejected) => return Ok(rejected(job)),
                    Err(_) => return Ok(JobOutcome::Unknown),
                };
                Ok(match receipt.outcome {
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
                })
            }
            Change::CreateOnly { source } | Change::Replace { source, .. } => {
                let precondition = match &action.change {
                    Change::Replace {
                        target_expected, ..
                    } => BridgePrecondition::Match(self.revision(target_expected)?),
                    _ => BridgePrecondition::MustNotExist,
                };
                match write_from_mirror(self.context, path, source, precondition, &job.external_key)
                {
                    Ok(receipt) => Ok(JobOutcome::Committed {
                        new_revision: Some(
                            self.context
                                .revisions
                                .borrow_mut()
                                .remember(receipt.revision),
                        ),
                        content_hash: Some(source.sha256),
                        bytes: Some(source.bytes),
                        receipt_id: receipt.revision.to_hex(),
                    }),
                    Err(BridgeSeamError::Rejected) => Ok(rejected(job)),
                    Err(_) => Ok(JobOutcome::Unknown),
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
        self.context
            .plane
            .transport()
            .lock()
            .map_err(|_| PlaneError::Unavailable)?
            .verify_quiescent(self.ticket)
            .map_err(|_| PlaneError::Quiescence)?;
        let path = &action.key.path;
        let view = &self.context.view;
        match &action.change {
            Change::CreateOnly { source } | Change::Replace { source, .. } => {
                if self.context.mode == SyncMode::MirrorRemote {
                    // 出站动作的源就是 canonical 当前版本；引导期它会被下一次 RunOnce 回滚，先保全。
                    preserve_canonical(
                        self.context,
                        naming::ROLLBACK_DIR,
                        path,
                        source,
                        &format!("{}/rollback", job.external_key),
                    )?;
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
                    Err(PlaneError::Observation) => rejected(job),
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
                    Err(PlaneError::Observation) => rejected(job),
                    Err(_) => JobOutcome::Unknown,
                })
            }
        }
    }
}

/// 权威 CAS 拒绝的固定无效果回执；终态标识由原键派生，不带路径。
fn rejected(job: &IssuedJob) -> JobOutcome {
    JobOutcome::NoEffect {
        reason: NoEffectCause::CasRejected,
        terminal_id: format!("cas-rejected:{}", hex(&sha256(job.external_key.as_bytes()))),
    }
}

impl JobBackend for PlaneJobBackend<'_> {
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome {
        let result = match action.key.target {
            Side::Canonical => self.apply_canonical(job, action),
            Side::Mirror => self.apply_mirror(job, action, staged),
        };
        // 任何失败都只能是未知：执行器不重发，下一轮按原键核对。
        result.unwrap_or(JobOutcome::Unknown)
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
