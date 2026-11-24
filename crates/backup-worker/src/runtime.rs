//! 固定包、凭据、仓库及进程回收共用一条边界；不含正文源路径或重开能力。
//! 连仓库的每一次调用都在这里加上批准的 CA（`--cacert`）：子进程环境是清空的，
//! 自签证书的仓库在系统信任库里没有，不加就连不上；加在这一层，任何调用都漏不掉。
use crate::{
    ProcessError, ResticCredentials, ResticPackage, Result, WorkerConfig, process,
    source::window::{Window, utc_now},
    temporary::Temporary,
};
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use studiolo_backup::worker::restic::{RESTIC_PACKAGE_VERSION, RepositoryRef};
use studiolo_core::checkpoint::InstantMillis;

pub(crate) struct Runtime {
    package: ResticPackage,
    credentials: ResticCredentials,
    temporary: Temporary,
    repository: RepositoryRef,
    /// 摘要核对过的仓库 CA 文件；配置没批准时为 `None`，用系统信任库。
    repository_ca: Option<&'static str>,
    window: Window,
    poisoned: Cell<bool>,
}
impl Runtime {
    pub fn open(
        config: &WorkerConfig,
        credentials: ResticCredentials,
        deadline: Instant,
    ) -> Result<Self> {
        let started = Instant::now();
        let now = utc_now()?;
        let expiry = now
            .millis()
            .checked_add(7_200_000)
            .ok_or(ProcessError::Deadline)?;
        let window = Window::new(
            deadline,
            started,
            now,
            now,
            InstantMillis::from_millis(expiry),
        )?;
        let package = ResticPackage::open(config.restic_sha256(), deadline)?;
        let temporary = Temporary::open()?;
        let version = process::run_scoped(
            &package,
            &["version".into()],
            None,
            temporary.path(),
            deadline.min(started + Duration::from_secs(10)),
            &|| window.check(),
        )?;
        let prefix = format!("restic {RESTIC_PACKAGE_VERSION} ");
        if !version.success || !version.stdout.starts_with(prefix.as_bytes()) {
            return Err(ProcessError::Protocol);
        }
        temporary.check()?;
        credentials.check(deadline)?;
        Ok(Self {
            package,
            credentials,
            temporary,
            repository: config.repository().clone(),
            repository_ca: config.repository_ca(),
            window,
            poisoned: Cell::new(false),
        })
    }
    /// 交给 restic 的完整参数：批准了 CA 就在最前面加 `--cacert <路径>`（restic 的全局参数）。
    fn arguments(&self, argv: &[String]) -> Vec<String> {
        with_ca(self.repository_ca, argv)
    }
    pub fn capture(
        &self,
        argv: &[String],
        input: Option<&[u8]>,
        deadline: Instant,
        source_time: &impl Fn() -> Result<()>,
    ) -> Result<process::Output> {
        self.check()?;
        let check = || {
            self.window.check()?;
            source_time()
        };
        let deadline = deadline.min(self.window.deadline());
        let argv = self.arguments(argv);
        let result = match input {
            Some(bytes) => process::run_input(
                &self.package,
                &argv,
                &self.credentials,
                self.temporary.path(),
                deadline,
                &check,
                bytes,
            ),
            None => process::run_scoped(
                &self.package,
                &argv,
                Some(&self.credentials),
                self.temporary.path(),
                deadline,
                &check,
            ),
        };
        self.after(result)
    }
    pub fn digest(
        &self,
        argv: &[String],
        expected: process::DigestExpectation<'_>,
        deadline: Instant,
        source_time: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.check()?;
        let check = || {
            self.window.check()?;
            source_time()
        };
        let result = process::verify_digest(
            &self.package,
            &self.arguments(argv),
            &self.credentials,
            self.temporary.path(),
            deadline.min(self.window.deadline()),
            &check,
            expected,
        );
        self.after(result)
    }
    /// 整份封存一次读回，执行边界和 [`Runtime::digest`] 相同；见 [`process::verify_tree`]。
    pub fn verify_tree(
        &self,
        argv: &[String],
        expected: &process::TreeExpectation,
        deadline: Instant,
        source_time: &impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.check()?;
        let check = || {
            self.window.check()?;
            source_time()
        };
        let result = process::verify_tree(
            &self.package,
            &self.arguments(argv),
            &self.credentials,
            self.temporary.path(),
            deadline.min(self.window.deadline()),
            &check,
            expected,
        );
        self.after(result)
    }
    /// 仓库抽查：同一条执行边界，截止时间就是打开时给的窗口；`interrupt` 见 [`process::run_check`]。
    pub fn check_subset(
        &self,
        argv: &[String],
        interrupt: &dyn Fn() -> Result<()>,
    ) -> Result<process::Output> {
        self.check()?;
        let result = process::run_check(
            &self.package,
            &self.arguments(argv),
            &self.credentials,
            self.temporary.path(),
            self.window.deadline(),
            &|| self.window.check(),
            interrupt,
        );
        self.after(result)
    }
    fn after<T>(&self, result: Result<T>) -> Result<T> {
        if matches!(result, Err(ProcessError::CleanupFailed)) {
            self.poisoned.set(true);
        }
        let value = result?;
        self.check()?;
        Ok(value)
    }
    fn check(&self) -> Result<()> {
        if self.poisoned.get() {
            return Err(ProcessError::CleanupFailed);
        }
        self.window.check()?;
        self.temporary.check()
    }
    pub fn now(&self) -> Result<InstantMillis> {
        self.check()?;
        self.window.current_time()
    }
    pub fn deadline(&self) -> Instant {
        self.window.deadline()
    }
    pub fn repository(&self) -> &RepositoryRef {
        &self.repository
    }
    pub fn poisoned(&self) -> bool {
        self.poisoned.get()
    }
}

/// 纯逻辑：批准了 CA 就把 `--cacert <路径>` 放在参数最前面，其余参数原样不动。
fn with_ca(ca: Option<&str>, argv: &[String]) -> Vec<String> {
    ca.map(|path| ["--cacert".to_owned(), path.to_owned()])
        .into_iter()
        .flatten()
        .chain(argv.iter().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::with_ca;

    #[test]
    fn an_approved_ca_is_passed_first_and_the_rest_is_untouched() {
        let argv = vec!["--json".to_owned(), "snapshots".to_owned()];
        assert_eq!(with_ca(None, &argv), argv);
        assert_eq!(
            with_ca(Some("/etc/studiolo/backup-worker-ca.pem"), &argv),
            [
                "--cacert",
                "/etc/studiolo/backup-worker-ca.pem",
                "--json",
                "snapshots"
            ]
        );
    }
}
