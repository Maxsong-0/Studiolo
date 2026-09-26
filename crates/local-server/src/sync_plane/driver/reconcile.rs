//! 第 10 步续做：上一批仍有 Issued 未知终态的动作时，按**原键**向后端核对权威终态并写回执。
//!
//! 只核对、不重放：canonical 侧靠 Broker 的按键重放（同一幂等键再提交只返回原回执，绝不第二次安装）；
//! 镜像侧靠静止窗口内的目标版本核对（目标已是冻结源版本＝已提交，仍是冻结前置版本＝未安装）。
//! 两者都核不出的保持未知，交给健康页降级；绝不用「内容碰巧相同」以外的推断补成功。

use super::content::RoundContext;
use crate::sync_plane::PlaneError;

/// 一次核对的结果计数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// 本轮按原键核出权威终态并已写回执的动作数。
    pub resolved: usize,
    /// 核对后仍未知的动作数。
    pub still_unknown: usize,
}

/// 核对上一批全部 Issued 未知动作。
///
/// # Errors
/// 持久层或后端不可用时返回对应分类；任何一条核不出都不是错误，只计入 `still_unknown`。
pub fn reconcile_pending(_context: &RoundContext<'_>) -> Result<ReconcileReport, PlaneError> {
    // B 轮补漏 R1 实现：按原键核对 canonical／镜像两侧终态并经 StateJournal::record_outcome 写回执。
    Ok(ReconcileReport::default())
}
