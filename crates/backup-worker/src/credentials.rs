//! 凭据不实现 Debug/Clone；只向固定子进程的三个环境槽位借出。
mod acl;
mod fault;
mod file;
mod input;
mod mount;
mod root;
use crate::{ProcessError, Result, WorkerConfig};
pub use fault::CredentialFault;
#[cfg(test)]
use std::path::Path;
use std::{process::Command, time::Instant};
use studiolo_process_safety::Residency;
use zeroize::Zeroizing;

/// 仅在 worker 已加固并记录驻留面结果之后，从固定系统凭据文件构造。
pub struct ResticCredentials {
    password_file: Zeroizing<String>,
    username: Zeroizing<String>,
    password: Zeroizing<String>,
    origin: Origin,
}
enum Origin {
    Fixed(Box<input::Input>),
    #[cfg(test)]
    Fixture,
}
impl ResticCredentials {
    /// 只读固定 systemd 凭据根的三个槽位；不接受环境或请求中的路径/凭据。
    /// 取不到时回固定类别（哪一道检查没过），调用方打进日志；类别里没有路径和内容。
    pub fn load(
        _residency: &Residency,
        config: &WorkerConfig,
        deadline: Instant,
    ) -> std::result::Result<Self, CredentialFault> {
        let values = input::Input::load(config.worker_uid(), deadline)?;
        Ok(Self {
            password_file: Zeroizing::new(format!("{}/repository-password", root::ROOT)),
            username: values.username,
            password: values.password,
            origin: Origin::Fixed(Box::new(values.input)),
        })
    }
    pub(crate) fn attach(&self, command: &mut Command, deadline: Instant) -> Result<()> {
        self.check(deadline)?;
        command
            .env("RESTIC_PASSWORD_FILE", self.password_file.as_str())
            .env("RESTIC_REST_USERNAME", self.username.as_str())
            .env("RESTIC_REST_PASSWORD", self.password.as_str());
        Ok(())
    }
    pub(crate) fn check(&self, deadline: Instant) -> Result<()> {
        match &self.origin {
            Origin::Fixed(input) => input.check(deadline).map_err(ProcessError::from),
            #[cfg(test)]
            Origin::Fixture => Ok(()),
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(path: &Path) -> Self {
        Self {
            password_file: Zeroizing::new(path.to_str().unwrap().into()),
            username: Zeroizing::new("fixture".into()),
            password: Zeroizing::new("fictional-rest-password".into()),
            origin: Origin::Fixture,
        }
    }
}
