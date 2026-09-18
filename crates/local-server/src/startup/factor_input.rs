//! 唯一服务器秘密装配边界；固定环境槽、纯共用解码和自有缓冲零化。
//!
//! 秘密驻留面的收紧**不在这里**：它已提到 `startup::prepare_composed` 的第一条语句，
//! 与是否启用因子模式无关（见那里的注释）。本文件从前只禁 core，而且埋在这条
//! 只有因子模式才会走的路径上，非因子模式因此一项也没有；现在这里只负责读槽位、
//! 解码与零化，绝不再自持任何一份部分收紧，否则就成了「加固两次只记录一次」。

use super::StartupError;
use crate::options::FactorOptions;
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::Path, sync::Arc};
use studiolo_auth::{FactorRuntime, SessionError, SessionManager, TotpSeed};
use studiolo_host_config::Config;
use studiolo_local_server::PreparedUnixIngress;
use zeroize::Zeroizing;

const CURRENT: &str = "STUDIOLO_TOTP_SEED_HEX";
const REPLACEMENT: &str = "STUDIOLO_TOTP_REPLACEMENT_SEED_HEX";

pub(super) struct Inputs {
    current: TotpSeed,
    replacement: Option<TotpSeed>,
    ingress: PreparedUnixIngress,
}

pub(super) struct Prepared {
    pub(super) runtime: Arc<FactorRuntime>,
    pub(super) ingress: PreparedUnixIngress,
}

pub(super) type Environment = (Option<OsString>, Option<OsString>);

pub(super) fn environment() -> Environment {
    (std::env::var_os(CURRENT), std::env::var_os(REPLACEMENT))
}

impl Inputs {
    pub(super) fn load(
        options: &FactorOptions,
        config: &Config,
        config_path: &Path,
        vault: &Path,
        assets: &Path,
        writer_data: Option<&Path>,
        environment: impl FnOnce() -> Environment,
    ) -> Result<Self, StartupError> {
        let config_directory = config_path.parent().ok_or(StartupError::Configuration)?;
        let mut isolated_roots = vec![
            config.auth_directory(),
            config.audit_directory(),
            vault,
            assets,
            config_directory,
        ];
        isolated_roots.extend(writer_data);
        let ingress = PreparedUnixIngress::prepare(&options.socket, &isolated_roots)
            .map_err(|_| StartupError::Ingress)?;
        // 组合根已经在读取任何秘密之前收紧过本进程，这里直接读槽位。
        let (current, replacement) = environment();
        let (current, replacement) = slots(
            config.seed_ref(),
            options.replacement_seed_ref.as_deref(),
            current,
            replacement,
        )?;
        Ok(Self {
            current,
            replacement,
            ingress,
        })
    }

    pub(super) fn attach(self, sessions: Arc<SessionManager>) -> Result<Prepared, StartupError> {
        let runtime = Arc::new(FactorRuntime::attach(
            sessions,
            self.current,
            self.replacement,
        )?);
        Ok(Prepared {
            runtime,
            ingress: self.ingress,
        })
    }
}

fn slots(
    current_ref: &str,
    replacement_ref: Option<&str>,
    current: Option<OsString>,
    replacement: Option<OsString>,
) -> Result<(TotpSeed, Option<TotpSeed>), StartupError> {
    // 两个自有环境副本都先零化封装，后续任何配置/解析失败也不会遗留普通Vec。
    let current = current.map(|value| Zeroizing::new(value.into_vec()));
    let replacement = replacement.map(|value| Zeroizing::new(value.into_vec()));
    if replacement_ref.is_some() != replacement.is_some() {
        return Err(StartupError::FactorEnvironment);
    }
    let current = TotpSeed::from_hex(
        current_ref.to_owned(),
        current.as_deref().ok_or(StartupError::FactorEnvironment)?,
    )
    .map_err(seed_error)?;
    let replacement = replacement_ref
        .zip(replacement)
        .map(|(reference, bytes)| {
            TotpSeed::from_hex(reference.to_owned(), &bytes).map_err(seed_error)
        })
        .transpose()?;
    Ok((current, replacement))
}

fn seed_error(error: SessionError) -> StartupError {
    match error {
        SessionError::Rejected => StartupError::FactorEnvironment,
        _ => StartupError::Configuration,
    }
}

#[cfg(test)]
#[path = "../../tests/factor_startup/environment.rs"]
mod tests;
