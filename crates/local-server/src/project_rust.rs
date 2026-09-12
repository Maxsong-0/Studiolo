//! 显式源码白名单的稳定观测；正文仅在本次内存中，不提供源码读取端点。

mod collection;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadNote, ReadOnlyVault};
use studiolo_workbench::{
    RustCallGraph, RustSourceError, RustSourceErrorKind, derive_rust_call_sites,
    validate_rust_source_paths,
};

use crate::LocalError;

#[derive(Serialize)]
pub(crate) struct RustCallObservation {
    pub status: &'static str,
    pub observed_at: Option<String>,
    pub graph: Option<RustCallGraph>,
    pub diagnostics: Vec<String>,
}

impl RustCallObservation {
    pub(crate) fn disabled() -> Self {
        Self::empty("disabled", "未启用 Rust 源码白名单读取。")
    }

    pub(crate) fn unbound() -> Self {
        Self::empty("unbound", "当前项目未绑定仓库，Rust 调用位置证据不可用。")
    }

    fn empty(status: &'static str, message: &str) -> Self {
        Self {
            status,
            observed_at: None,
            graph: None,
            diagnostics: vec![message.to_owned()],
        }
    }
}

pub(crate) fn observe(
    root: &ReadOnlyVault,
    paths: &[String],
    revalidate: impl FnMut() -> Result<(), LocalError>,
) -> RustCallObservation {
    observe_with(paths, |path, limit| root.read_file(path, limit), revalidate)
}

// 私有接缝验证完整计划与两轮变化；生产只接受同一固定根 Broker 的读取结果。
fn observe_with(
    paths: &[String],
    mut read: impl FnMut(&VaultPath, u64) -> Result<ReadNote, BrokerError>,
    mut revalidate: impl FnMut() -> Result<(), LocalError>,
) -> RustCallObservation {
    let result = (|| {
        // 即便可信装配已经验证，也在任何读取前完整检查一次白名单。
        let paths = validate_rust_source_paths(paths).map_err(Failure::from)?;
        let paths = paths
            .iter()
            .map(|path| {
                VaultPath::parse(path).map_err(|_| Failure::invalid("Rust 源码白名单路径无效。"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        revalidate().map_err(|_| Failure::unavailable("仓库根绑定已变化或不可读。"))?;
        let first = collection::capture(&mut read, &paths, false)?;
        let second = collection::capture(&mut read, &paths, true)?;
        if first.len() != second.len()
            || first
                .iter()
                .zip(&second)
                .any(|(before, after)| !before.same(after))
        {
            return Err(Failure::unavailable(
                "Rust 源码在两轮观测之间变化，请重新读取。",
            ));
        }
        let inputs = first.into_iter().map(|item| item.input).collect::<Vec<_>>();
        let graph = derive_rust_call_sites(&inputs).map_err(Failure::from)?;
        revalidate().map_err(|_| Failure::unavailable("仓库根绑定已变化或不可读。"))?;
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Failure::unavailable("无法记录本次 Rust 源码观测时间。"))?
            .as_secs()
            .to_string();
        Ok((graph, observed_at))
    })();
    match result {
        Ok((graph, observed_at)) => RustCallObservation {
            status: "ready",
            observed_at: Some(observed_at),
            graph: Some(graph),
            diagnostics: Vec::new(),
        },
        Err(error) => RustCallObservation::empty(error.status, &error.message),
    }
}

struct Failure {
    status: &'static str,
    message: String,
}

impl Failure {
    fn unavailable(message: &str) -> Self {
        Self {
            status: "unavailable",
            message: message.to_owned(),
        }
    }

    fn invalid(message: &str) -> Self {
        Self {
            status: "invalid",
            message: message.to_owned(),
        }
    }
}

impl From<RustSourceError> for Failure {
    fn from(error: RustSourceError) -> Self {
        Self {
            status: match error.kind {
                RustSourceErrorKind::Invalid => "invalid",
                RustSourceErrorKind::Unsupported => "unsupported",
            },
            // 纯层仅提供受控原因，禁止原始解析错误和源码正文进入诊断。
            message: error.message,
        }
    }
}

#[cfg(test)]
#[path = "../tests/project_rust/budgets.rs"]
mod budget_tests;
#[cfg(test)]
#[path = "../tests/project_rust/reading.rs"]
mod reading_tests;
#[cfg(test)]
#[path = "../tests/project_rust/security.rs"]
mod security_tests;
#[cfg(test)]
#[path = "../tests/project_rust/support.rs"]
mod test_support;
