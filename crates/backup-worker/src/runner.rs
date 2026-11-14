//! 追加角色只接受 HTTPS REST 仓库和 Backup/Check 的封闭计划；其余身份另行装配。
pub(crate) mod final_record;
pub(crate) mod inspect;
mod probe;
use crate::{
    ApprovedJob, ProcessError, ResticCredentials, Result, process, runtime::Runtime, source::Source,
};
pub use final_record::FinalManifestProbe;
use std::cell::RefCell;
use studiolo_backup::worker::{
    final_manifest::FinalManifestDraft,
    restic::{RepositoryRef, ResticError, ResticInvocation, ResticOutcome, ResticRunner},
};
use studiolo_backup::{checkpoint::staging::SealedCheckpoint, worker::state::JobKey};

/// 单个作业持有固定配置、包、追加凭据与真实封存；启动方另负责秘密来源及作业租约。
pub struct AppendRunner {
    runtime: Runtime,
    source: Source,
    verified: RefCell<Option<FinalManifestDraft>>,
}
impl AppendRunner {
    /// 所有调用共用原作业截止时间，检查和上传不会各自重新得到完整预算。
    pub fn new(job: &ApprovedJob<'_>, credentials: ResticCredentials) -> Result<Self> {
        job.check_live()?;
        let source = Source::open(
            job.config(),
            job.key().clone(),
            job.fact().source(),
            job.deadline(),
        )?;
        job.fact()
            .verify_sealed(source.sealed())
            .map_err(|_| ProcessError::Source)?;
        let runtime = Runtime::open(job.config(), credentials, source.deadline())?;
        source.check()?;
        job.check_live()?;
        Ok(Self {
            runtime,
            source,
            verified: RefCell::new(None),
        })
    }

    /// 返回固定错误分类；CleanupFailed 须令外层停止服务，不能当成可重试上传。
    pub fn execute(&self, invocation: &ResticInvocation) -> Result<ResticOutcome> {
        let backup = self.source.accepts(invocation)?;
        self.source.check()?;
        self.verified.borrow_mut().take();
        let source_time = || self.source.check_time();
        let output = self.runtime.capture(
            invocation.argv(),
            None,
            self.source.deadline(),
            &source_time,
        )?;
        self.source.check()?;
        parse_output(&output, backup)
    }
    /// 已核验封存，只能用于此 runner 的同一作业。
    #[must_use]
    pub fn sealed(&self) -> &SealedCheckpoint {
        self.source.sealed()
    }
    /// 封存与所有调用固定绑定的原作业键。
    #[must_use]
    pub fn key(&self) -> &JobKey {
        self.source.key()
    }
    /// 任何清理未证实都会永久封闭本 runner；外层须停止整个 worker 服务。
    #[must_use]
    pub fn requires_shutdown(&self) -> bool {
        self.runtime.poisoned()
    }
}
impl ResticRunner for AppendRunner {
    fn run(
        &self,
        invocation: &ResticInvocation,
    ) -> std::result::Result<ResticOutcome, ResticError> {
        self.execute(invocation)
            .map_err(|_| ResticError::ExecutionFailed)
    }
}
pub(crate) fn parse_output(output: &process::Output, backup: bool) -> Result<ResticOutcome> {
    if !output.success {
        return Ok(ResticOutcome::Exited {
            status_zero: false,
            reported_objects: 0,
        });
    }
    let mut count = 0;
    let mut summaries = 0;
    for line in output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value =
            serde_json::from_slice(line).map_err(|_| ProcessError::Protocol)?;
        if backup && value.get("message_type").and_then(|v| v.as_str()) == Some("summary") {
            summaries += 1;
            count = value
                .get("total_files_processed")
                .and_then(|v| v.as_u64())
                .ok_or(ProcessError::Protocol)?;
            let snapshot = value
                .get("snapshot_id")
                .and_then(|v| v.as_str())
                .ok_or(ProcessError::Protocol)?;
            if snapshot.len() != 64
                || !snapshot
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(ProcessError::Protocol);
            }
        }
    }
    if backup && summaries != 1 {
        return Err(ProcessError::Protocol);
    }
    Ok(ResticOutcome::Exited {
        status_zero: true,
        reported_objects: count,
    })
}
pub(crate) fn validate_repository(repository: &RepositoryRef) -> Result<()> {
    let text = repository
        .as_str()
        .strip_prefix("rest:")
        .ok_or(ProcessError::Configuration)?;
    let url = url::Url::parse(text).map_err(|_| ProcessError::Configuration)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || text.contains(['@', '\\'])
        || url.as_str() != text
    {
        return Err(ProcessError::Configuration);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn append_backend_rejects_helpers_userinfo_and_ambiguous_urls() {
        for repository in [
            "/tmp/local",
            "sftp:host:/repo",
            "s3:https://host/repo",
            "rest:https://user:pass@host/repo",
            "rest:https://host/repo?token=x",
            "rest:https://host/repo#fragment",
            "rest:https://host/../other",
            "rest:https://host:443/repo",
        ] {
            let result = RepositoryRef::parse(repository)
                .map_err(|_| ProcessError::Configuration)
                .and_then(|r| validate_repository(&r));
            assert_eq!(result, Err(ProcessError::Configuration));
        }
        assert!(
            validate_repository(
                &RepositoryRef::parse("rest:https://backup.example.test/repository/").unwrap()
            )
            .is_ok()
        );
    }
}
