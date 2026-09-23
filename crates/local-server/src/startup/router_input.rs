//! 按已核验的入口能力选出唯一的路由组合；装配失败只带固定类别，并先关停已启动的写者。

use super::{StartupError, canonical_input};
use std::sync::Arc;
use studiolo_auth::{AccessVerifier, FactorRuntime, SessionManager};
use studiolo_local_server::{CanonicalWorkbench, LocalError};
use studiolo_vault_broker::{CanonicalReadVault, ReadOnlyVault};

/// 选择路由所需的全部输入；每一项都已由可信启动层核验，没有一项来自 HTTP。
pub(super) struct Inputs {
    pub(super) vault: ReadOnlyVault,
    pub(super) assets: ReadOnlyVault,
    pub(super) label: String,
    pub(super) port: u16,
    /// 已取钥的 Access 验证器；缺席即本机只读模式。
    pub(super) access: Option<AccessVerifier>,
    pub(super) sessions: Option<Arc<SessionManager>>,
    pub(super) factors: Option<Arc<FactorRuntime>>,
    pub(super) canonical: Option<Arc<CanonicalReadVault>>,
    /// 已就绪的 canonical 写者；装配失败时由这里负责关停。
    pub(super) workbench: Option<Arc<CanonicalWorkbench>>,
}

/// 装配唯一组合并返回模式说明；失败时先关停写者，再返回不含秘密的原因类别。
pub(super) async fn compose(inputs: Inputs) -> Result<(axum::Router, &'static str), StartupError> {
    let workbench = inputs.workbench.clone();
    let (app, mode) = select(inputs);
    match app {
        Ok(app) => Ok((app, mode)),
        Err(error) => {
            canonical_input::shutdown(workbench.as_ref()).await?;
            Err(StartupError::router(&error))
        }
    }
}

/// 组合优先级：写者 > 因子 > canonical 只读 > 普通会话；无会话时只有 Access 或本机只读。
fn select(inputs: Inputs) -> (Result<axum::Router, LocalError>, &'static str) {
    let Inputs {
        vault,
        assets,
        label,
        port,
        access,
        sessions,
        factors,
        canonical,
        workbench,
    } = inputs;
    let Some(access) = access else {
        return (
            studiolo_local_server::router(vault, assets, label, port),
            "本机只读",
        );
    };
    let Some(sessions) = sessions else {
        return (
            studiolo_local_server::router_with_access(vault, assets, label, port, access),
            "公共示例入口已启用Access鉴权；只读",
        );
    };
    if let Some(workbench) = workbench {
        return (
            studiolo_local_server::router_with_canonical_writer(
                workbench, assets, label, port, access, sessions, factors,
            ),
            "Access及应用会话；canonical完整监视只读",
        );
    }
    if let Some(factors) = factors {
        return (
            studiolo_local_server::router_with_factors(
                (vault, canonical),
                assets,
                label,
                access,
                sessions,
                factors,
            ),
            "Access、应用会话及二次认证；可信Unix入口",
        );
    }
    if let Some(canonical) = canonical {
        return (
            studiolo_local_server::router_with_canonical_sessions(
                (vault, canonical),
                assets,
                label,
                port,
                access,
                sessions,
            ),
            "Access及已登记应用会话；canonical只读",
        );
    }
    (
        studiolo_local_server::router_with_sessions(vault, assets, label, port, access, sessions),
        "Access及已登记应用会话；只读",
    )
}
