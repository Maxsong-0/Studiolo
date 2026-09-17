//! 显式授权后的固定清单观测；正文只留在本次内存中，不提供文件读取端点。

mod collection;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use studiolo_core::VaultPath;
use studiolo_vault_broker::{BrokerError, ReadNote, ReadOnlyVault};
use studiolo_workbench::{
    CargoDependencyError, CargoDependencyErrorKind, CargoDependencyGraph, derive_cargo_dependencies,
};

use crate::LocalError;

#[derive(Serialize)]
pub(crate) struct CargoDependencyObservation {
    pub status: &'static str,
    pub observed_at: Option<String>,
    pub graph: Option<CargoDependencyGraph>,
    pub diagnostics: Vec<String>,
}

impl CargoDependencyObservation {
    pub(crate) fn disabled() -> Self {
        Self::empty(
            "disabled",
            "未启用 Cargo 清单读取；当前仅有原目录观测权限。",
        )
    }

    pub(crate) fn unbound() -> Self {
        Self::empty("unbound", "当前项目未绑定仓库，Cargo 清单证据不可用。")
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
    revalidate: impl FnMut() -> Result<(), LocalError>,
) -> CargoDependencyObservation {
    observe_with(|path, limit| root.read_file(path, limit), revalidate)
}

// 私有读取接缝便于确定性验证两轮之间的文件变动；生产只传固定根 Broker 读取。
fn observe_with(
    mut read: impl FnMut(&VaultPath, u64) -> Result<ReadNote, BrokerError>,
    mut revalidate: impl FnMut() -> Result<(), LocalError>,
) -> CargoDependencyObservation {
    let result = (|| {
        revalidate().map_err(|_| Failure::unavailable("仓库根绑定已变化或不可读。"))?;
        let first = collection::capture(&mut read, None)?;
        let second = collection::capture(&mut read, Some(&first))?;
        if first.len() != second.len()
            || first
                .iter()
                .zip(&second)
                .any(|(before, after)| !before.same(after))
        {
            return Err(Failure::unavailable(
                "Cargo 清单在两轮观测之间变化，请重新读取。",
            ));
        }
        let manifests = first
            .into_iter()
            .map(|item| item.manifest)
            .collect::<Vec<_>>();
        let graph = derive_cargo_dependencies(&manifests).map_err(Failure::from)?;
        revalidate().map_err(|_| Failure::unavailable("仓库根绑定已变化或不可读。"))?;
        let observed_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Failure::unavailable("无法记录本次 Cargo 清单观测时间。"))?
            .as_secs()
            .to_string();
        Ok((graph, observed_at))
    })();
    match result {
        Ok((graph, observed_at)) => CargoDependencyObservation {
            status: "ready",
            observed_at: Some(observed_at),
            graph: Some(graph),
            diagnostics: Vec::new(),
        },
        Err(error) => CargoDependencyObservation::empty(error.status, &error.message),
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

impl From<CargoDependencyError> for Failure {
    fn from(error: CargoDependencyError) -> Self {
        Self {
            status: match error.kind {
                CargoDependencyErrorKind::Invalid => "invalid",
                CargoDependencyErrorKind::Unsupported => "unsupported",
            },
            // 纯层只返回受控原因，绝不传播 TOML 解析器原始错误或清单正文。
            message: error.message,
        }
    }
}

#[cfg(test)]
#[path = "../tests/project_cargo/support.rs"]
mod test_support;

#[cfg(test)]
#[path = "../tests/project_cargo/reading.rs"]
mod reading_tests;

#[cfg(test)]
#[path = "../tests/project_cargo/budgets.rs"]
mod budget_tests;

#[cfg(test)]
#[path = "../tests/project_cargo/security.rs"]
mod security_tests;
