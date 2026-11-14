//! 固定包、凭据、仓库及进程回收共用一条边界；不含正文源路径或重开能力。
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
            window,
            poisoned: Cell::new(false),
        })
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
        let result = match input {
            Some(bytes) => process::run_input(
                &self.package,
                argv,
                &self.credentials,
                self.temporary.path(),
                deadline,
                &check,
                bytes,
            ),
            None => process::run_scoped(
                &self.package,
                argv,
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
            argv,
            &self.credentials,
            self.temporary.path(),
            deadline.min(self.window.deadline()),
            &check,
            expected,
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
