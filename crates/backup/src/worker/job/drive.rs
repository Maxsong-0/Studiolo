//! 常驻循环对一个原键做的那一步：只按本地位置和来源是否还在有效期决定走哪条已有的路。
//!
//! 这里不碰远端、不读配置、不开封存：那些由进程一侧装配成 [`JobPorts`] 交进来。
//! 决策表只有一张，放在状态机这一层，进程入口和测试共用同一份。
use super::{BackupWorker, DataRemote, JobOutcome, JobPosition, RemoteProbe, WorkerError};
use crate::{
    checkpoint::staging::SealedCheckpoint,
    worker::{
        final_manifest::{FinalManifestReader, FinalManifestWriter},
        restic::ResticRunner,
        state::JobKey,
    },
};

/// 已经重开并核对过的原封存，加只为这一个作业装配的数据面。
pub trait Transfer {
    /// 原封存；不是调用方另传的路径。
    fn sealed(&self) -> &SealedCheckpoint;
    /// 只接受封闭上传计划的执行器。
    fn runner(&self) -> &dyn ResticRunner;
    /// 沿原作业键核对远端数据快照。
    fn probe(&self) -> &dyn RemoteProbe;
    /// 最终清单的提交和读回。
    fn finalizer(&self) -> &dyn FinalManifestWriter;
}

/// 这一轮外部能力装配不起来；固定类别，不带细节。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortUnavailable;

/// 进程一侧为一个原键装配的外部能力。
pub trait JobPorts {
    /// 原封存是否还在有效期内；由原准入事实和可信时钟判断，不由本地状态猜。
    fn source_live(&self) -> bool;
    /// 重开原封存并装配数据面。打不开就返回错误，此时本地状态不变。
    fn transfer(&self) -> Result<Box<dyn Transfer + '_>, PortUnavailable>;
    /// 最终清单的只读核对端口；不需要原封存，来源过期之后仍然可用。
    fn final_reader(&self) -> Result<Box<dyn FinalManifestReader + '_>, PortUnavailable>;
}

/// 这一步的结论。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    /// 状态机给出的结论；终态之外的都要下一轮再来。
    Outcome(JobOutcome),
    /// 外部能力这一轮装配不起来；除了可能刚接收了原键，本地状态没有变化。
    Deferred,
}

impl BackupWorker<'_> {
    /// 对生产者列为待办的原键做一步。
    ///
    /// | 本地位置 | 来源还有效 | 来源已过期 |
    /// |---|---|---|
    /// | 没有 | 接收原键，然后同「排队中」 | 接收原键，然后同「排队中」 |
    /// | 排队中 | 上传、复验、提交最终清单 | 记 `source_expired` 失败 |
    /// | 上传或复验没做完 | 只读核对原快照，不重传 | 记 `source_expired` 失败 |
    /// | 最终清单草稿已持久 | 只读核对最终清单并补记 | 同左（不需要正文） |
    /// | 失败结果没写进账本 | 原样补记 | 同左 |
    /// | 终态 | 拒绝：生产者不该还列它为待办 | 同左 |
    pub fn advance(&self, key: &JobKey, ports: &dyn JobPorts) -> Result<Advance, WorkerError> {
        let mut position = self.position(key)?;
        if position == JobPosition::Absent {
            self.accept(key)?;
            position = JobPosition::Queued;
        }
        let outcome = match position {
            JobPosition::Queued | JobPosition::Interrupted if !ports.source_live() => {
                self.expire(key)?
            }
            JobPosition::Queued => {
                let Ok(transfer) = ports.transfer() else {
                    return Ok(Advance::Deferred);
                };
                self.run(
                    key,
                    transfer.sealed(),
                    DataRemote {
                        runner: transfer.runner(),
                        probe: transfer.probe(),
                    },
                    transfer.finalizer(),
                )?
            }
            JobPosition::Interrupted => {
                let Ok(transfer) = ports.transfer() else {
                    return Ok(Advance::Deferred);
                };
                self.reconcile_upload(
                    key,
                    transfer.sealed(),
                    transfer.probe(),
                    transfer.finalizer(),
                )?
            }
            JobPosition::Finalizing => {
                let Ok(reader) = ports.final_reader() else {
                    return Ok(Advance::Deferred);
                };
                self.reconcile_final(key, reader.as_ref())?
            }
            JobPosition::FailurePending => self.backfill_failure(key)?,
            // 本地已经是终态、结果也写过账本，生产者却还列为待办：两边不是同一条历史。
            JobPosition::Finished | JobPosition::Absent => {
                return Err(WorkerError::EvidenceMismatch);
            }
        };
        Ok(Advance::Outcome(outcome))
    }
}
