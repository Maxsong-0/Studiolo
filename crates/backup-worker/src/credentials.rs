//! 凭据不实现 Debug/Clone；只向固定子进程的三个环境槽位借出。
use crate::{ProcessError, Result};
use std::{
    path::{Component, Path},
    process::Command,
};
use studiolo_process_safety::Residency;
use zeroize::Zeroizing;

/// 仅在 worker 已加固并记录驻留面结果之后构造；配置和凭据文件的核验由启动装配负责。
pub struct ResticCredentials {
    password_file: Zeroizing<String>,
    username: Zeroizing<String>,
    password: Zeroizing<String>,
}
impl ResticCredentials {
    /// 输入须来自 worker 私有的固定凭据槽位，不得接受 HTTP 请求中的凭据。
    pub fn new(
        _residency: &Residency,
        password_file: String,
        username: String,
        password: String,
    ) -> Result<Self> {
        let this = Self {
            password_file: Zeroizing::new(password_file),
            username: Zeroizing::new(username),
            password: Zeroizing::new(password),
        };
        let path = Path::new(this.password_file.as_str());
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
            || this.password_file.len() > 4096
            || this.password_file.chars().any(char::is_control)
            || this.username.is_empty()
            || this.username.len() > 256
            || this.username.contains(':')
            || this.password.is_empty()
            || this.password.len() > 4096
            || this
                .username
                .chars()
                .chain(this.password.chars())
                .any(char::is_control)
        {
            return Err(ProcessError::Configuration);
        }
        Ok(this)
    }
    pub(crate) fn attach(&self, command: &mut Command) {
        command
            .env("RESTIC_PASSWORD_FILE", self.password_file.as_str())
            .env("RESTIC_REST_USERNAME", self.username.as_str())
            .env("RESTIC_REST_PASSWORD", self.password.as_str());
    }
    #[cfg(test)]
    pub(crate) fn fixture(path: &Path) -> Self {
        Self {
            password_file: Zeroizing::new(path.to_str().unwrap().into()),
            username: Zeroizing::new("fixture".into()),
            password: Zeroizing::new("fictional-rest-password".into()),
        }
    }
}
