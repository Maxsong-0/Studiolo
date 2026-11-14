//! 追加角色只接受 HTTPS REST 仓库和 Backup/Check 的封闭计划；其余身份另行装配。
use crate::{
    ProcessError, ResticCredentials, ResticPackage, Result, WorkerConfig, process,
    temporary::Temporary,
};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use studiolo_backup::worker::restic::{
    RESTIC_PACKAGE_VERSION, RepositoryRef, ResticError, ResticIdentity, ResticInvocation,
    ResticOutcome, ResticRunner,
};

/// 单个 worker 持有固定配置、包与追加凭据；启动方另负责秘密来源、封存引用与作业租约。
pub struct AppendRunner {
    package: ResticPackage,
    repository: RepositoryRef,
    credentials: ResticCredentials,
    deadline: Instant,
    poisoned: Cell<bool>,
    temporary: Temporary,
}
impl AppendRunner {
    /// 所有调用共用原作业截止时间，检查和上传不会各自重新得到完整预算。
    pub fn new(
        config: &WorkerConfig,
        credentials: ResticCredentials,
        deadline: Instant,
    ) -> Result<Self> {
        let repository = config.repository().clone();
        let package = ResticPackage::open(config.restic_sha256(), deadline)?;
        let temporary = Temporary::open()?;
        if deadline <= Instant::now() || deadline > Instant::now() + Duration::from_secs(7200) {
            return Err(ProcessError::Configuration);
        }
        let version = process::run(
            &package,
            &["version".into()],
            None,
            temporary.path(),
            deadline.min(Instant::now() + Duration::from_secs(10)),
        )?;
        let prefix = format!("restic {RESTIC_PACKAGE_VERSION} ");
        if !version.success || !version.stdout.starts_with(prefix.as_bytes()) {
            return Err(ProcessError::Protocol);
        }
        temporary.check()?;
        Ok(Self {
            package,
            repository,
            credentials,
            deadline,
            poisoned: Cell::new(false),
            temporary,
        })
    }
    /// 返回固定错误分类；CleanupFailed 须令外层停止服务，不能当成可重试上传。
    pub fn execute(&self, invocation: &ResticInvocation) -> Result<ResticOutcome> {
        if self.poisoned.get() {
            return Err(ProcessError::CleanupFailed);
        }
        if invocation.identity() != ResticIdentity::Append
            || invocation.repository() != &self.repository
            || invocation.version() != RESTIC_PACKAGE_VERSION
            || invocation.package() != "restic"
        {
            return Err(ProcessError::Configuration);
        }
        self.temporary.check()?;
        let output = process::run(
            &self.package,
            invocation.argv(),
            Some(&self.credentials),
            self.temporary.path(),
            self.deadline,
        );
        if matches!(output, Err(ProcessError::CleanupFailed)) {
            self.poisoned.set(true);
        }
        let output = output?;
        self.temporary.check()?;
        parse_output(
            &output,
            invocation
                .argv()
                .get(4)
                .is_some_and(|command| command == "backup"),
        )
    }
    /// 任何清理未证实都会永久封闭本 runner；外层须停止整个 worker 服务。
    #[must_use]
    pub fn requires_shutdown(&self) -> bool {
        self.poisoned.get()
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
