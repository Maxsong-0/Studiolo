//! 执行器闭集失败；没有一个分支可以被读成「大概成功了」。

use super::{
    cas::CasFault,
    journal::{JournalFault, WitnessFault},
    quiescence::QuiescenceFault,
    transfer::TransferFault,
};
use crate::FuseReason;

/// 整批或单动作被拒绝的原因；调用方不得把任何变体折叠成成功。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutorError {
    /// 已请求关停；保留未闭合原计划，不把未发出动作记作无效果。
    Stopping,
    /// 持久层删除熔断仍生效；本片没有解除能力，解除须走已登记的 L3 动作。
    Fused,
    /// 本批删除数或占原共同基线存活路径比例越过冻结门槛，整个计划阻断。
    DeletionFuseBlocked(FuseReason),
    /// 原计划序号已退休，永久拒绝；拒绝不代表它曾经成功。
    Retired,
    /// 冻结计划与持久快照不是同一批：序号、动作键或整批统计不一致。
    PlanMismatch,
    /// 没有取得可用静止证据；不搬运任何对象。
    QuiescenceMissing(QuiescenceFault),
    /// 无静止证据却存在删除动作；删除在任何情况下都不得先于静止。
    DeletionWithoutQuiescence,
    /// 任一侧观察不完整、缺显式 Missing，或删除缺少原共同基线。
    IncompleteObservation,
    /// 冲突路径上出现了可执行动作；冲突必须保留双方，不能被动作覆盖。
    ConflictHasAction,
    /// 精确整数统计不能表示。
    Overflow,
    /// 审计仍有待确认事件却无法推进；不得把未确认当作已确认。
    AuditStalled,
    /// MirrorCAS 拒绝了本次内容寻址落库。
    Cas(CasFault),
    /// 持久层端口失败；Unavailable 可能发生在提交之后，须沿原键重开核对。
    Journal(JournalFault),
    /// 可信观察 adapter 无法给出新鲜重验证据。
    Witness(WitnessFault),
    /// 正文来源或镜像 sink 失败。
    Transfer(TransferFault),
}
