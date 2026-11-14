//! 只有完整数据核验得到的原 draft 可以提交；独立只读探针可以处理源过期后的最终 ACK。
mod listing;
pub(crate) mod plan;
pub(crate) mod read;
use super::AppendRunner;
use crate::{ApprovedJob, ProcessError, ResticCredentials, Result, runtime::Runtime};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use studiolo_backup::worker::{
    audit::AdmissionFact,
    final_manifest::{
        FinalManifestDraft, FinalManifestLookup, FinalManifestReader, FinalManifestReceipt,
        FinalManifestWriter,
    },
    restic::{ResticError, ResticOutcome},
};

/// 独立的非正文核对能力。没有 Source，不能打开本地封存、读取远端数据文件或上传。
pub struct FinalManifestProbe {
    runtime: Runtime,
    admission: AdmissionFact,
    source_root: PathBuf,
}
impl FinalManifestProbe {
    /// 固定配置/包/凭据沿用同一执行边界，单次核对最多五分钟，不继承过期源权限。
    pub fn new(job: &ApprovedJob<'_>, credentials: ResticCredentials) -> Result<Self> {
        Ok(Self {
            runtime: Runtime::open(
                job.config(),
                credentials,
                job.deadline()
                    .min(Instant::now() + Duration::from_secs(300)),
            )?,
            admission: job.fact().clone(),
            source_root: studiolo_backup::checkpoint::staging::SealedCheckpoint::root_path(
                job.config().sealed_parent(),
                &job.key().checkpoint_id,
            ),
        })
    }
    /// 清理无法证实即永久关闭；启动方须终止服务。
    #[must_use]
    pub fn requires_shutdown(&self) -> bool {
        self.runtime.poisoned()
    }
}
fn read_remote(runtime: &Runtime, draft: &FinalManifestDraft) -> Result<FinalManifestLookup> {
    let found = read::read(
        draft,
        runtime.repository(),
        &|argv| runtime.capture(argv, None, runtime.deadline(), &|| Ok(())),
        &|argv, expected| runtime.digest(argv, expected, runtime.deadline(), &|| Ok(())),
    )?;
    let now = runtime.now()?;
    match found {
        Some(snapshot) => Ok(FinalManifestLookup::Verified(Box::new(
            FinalManifestReceipt::new(draft.clone(), snapshot, now)
                .map_err(|_| ProcessError::Protocol)?,
        ))),
        None => Ok(FinalManifestLookup::Absent { observed_at: now }),
    }
}
impl FinalManifestReader for FinalManifestProbe {
    fn read_final(
        &self,
        draft: &FinalManifestDraft,
    ) -> std::result::Result<FinalManifestLookup, ResticError> {
        self.admission
            .verify_draft(draft)
            .map_err(|_| ResticError::ExecutionFailed)?;
        if draft.source_root() != self.source_root {
            return Err(ResticError::ExecutionFailed);
        }
        read_remote(&self.runtime, draft).map_err(|_| ResticError::ExecutionFailed)
    }
}
impl FinalManifestReader for AppendRunner {
    fn read_final(
        &self,
        draft: &FinalManifestDraft,
    ) -> std::result::Result<FinalManifestLookup, ResticError> {
        if draft.key() != self.key() || draft.source() != self.sealed().reference() {
            return Err(ResticError::ExecutionFailed);
        }
        read_remote(&self.runtime, draft).map_err(|_| ResticError::ExecutionFailed)
    }
}
impl AppendRunner {
    fn commit_marker(&self, draft: &FinalManifestDraft) -> Result<FinalManifestReceipt> {
        self.source.check()?;
        // 消费本执行器自己产生的证明；调用方自造 DTO 不会赋予一次上传权限。
        let verified = self
            .verified
            .borrow_mut()
            .take()
            .ok_or(ProcessError::Source)?;
        if &verified != draft {
            return Err(ProcessError::Source);
        }
        if let FinalManifestLookup::Verified(receipt) = read_remote(&self.runtime, draft)? {
            return Ok(*receipt);
        }
        self.source.check_time()?;
        let marker = plan::Marker::for_job(self.key())?;
        let bytes = draft.encode().map_err(|_| ProcessError::Protocol)?;
        let output = self.runtime.capture(
            &marker.backup(self.runtime.repository()),
            Some(&bytes),
            self.source.deadline(),
            &|| self.source.check_time(),
        )?;
        if !matches!(
            super::parse_output(&output, true)?,
            ResticOutcome::Exited {
                status_zero: true,
                ..
            }
        ) {
            return Err(ProcessError::Protocol);
        }
        self.source.check_time()?;
        match read_remote(&self.runtime, draft)? {
            FinalManifestLookup::Verified(receipt) => Ok(*receipt),
            FinalManifestLookup::Absent { .. } => Err(ProcessError::Protocol),
        }
    }
}
impl FinalManifestWriter for AppendRunner {
    fn commit_final(
        &self,
        draft: &FinalManifestDraft,
    ) -> std::result::Result<FinalManifestReceipt, ResticError> {
        self.commit_marker(draft)
            .map_err(|_| ResticError::ExecutionFailed)
    }
}
