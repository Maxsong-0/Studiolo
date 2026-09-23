//! 执行 restic 调用的 Interface、一次调用的结果，以及未接入真实仓库时的 runner。

use super::{ResticError, ResticInvocation};
use serde::{Deserialize, Serialize};

/// 一次真实调用的结果。拿不到真实仓库时返回 `Unverified`，不伪装成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResticOutcome {
    /// 进程已退出；退出码只是中间证据。
    Exited {
        /// 是否以 0 退出。
        status_zero: bool,
        /// 自报的对象数。
        reported_objects: u64,
    },
    /// 明确未验证；不是成功也不是失败。
    Unverified {
        /// 未验证的原因。
        reason: UnverifiedReason,
    },
}

/// 未验证的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnverifiedReason {
    /// 尚无真实备份仓库。
    NoRealRepository,
    /// 备份目标不可达。
    TargetUnreachable,
    /// 目标的不可变能力无法证明。
    ImmutabilityUnprovable,
}

/// 执行 restic 调用的 Interface；本 crate 不实现真实执行。
pub trait ResticRunner {
    /// 执行一次已检查的调用。
    fn run(&self, invocation: &ResticInvocation) -> Result<ResticOutcome, ResticError>;
}

/// 没有接入真实仓库时使用的 runner：一律返回明确未验证。
#[derive(Clone, Copy, Debug, Default)]
pub struct UnconfiguredResticRunner;

impl ResticRunner for UnconfiguredResticRunner {
    fn run(&self, _invocation: &ResticInvocation) -> Result<ResticOutcome, ResticError> {
        Ok(ResticOutcome::Unverified {
            reason: UnverifiedReason::NoRealRepository,
        })
    }
}
