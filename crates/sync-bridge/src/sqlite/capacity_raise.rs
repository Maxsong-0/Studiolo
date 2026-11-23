//! 持久预算只升容量（BUG-047）：`sync-init` 时冻结的路径数与输出项上限随同步路径预算升上去。
//!
//! 冻结在旧预算的 pending／last／fuse 不动，照旧用它们自己的输入与预算重算（`validate::frozen_limits_fit`）；
//! 之后的新预留只认升级后的预算（`reservation` 仍要求输入预算与持久预算相等）。删除阈值与单文件上限不在此改。
use super::{BridgeError, BridgeState, model::*, validate::frozen_limits_fit};
use crate::Limits;

impl BridgeState {
    /// 把持久预算升到 `target`；已是目标值返回 `Ok(None)` 且不写库，升了返回原来的 `max_paths`。
    ///
    /// # Errors
    /// 不是纯粹的容量升级（任一容量字段变小、删除阈值或单文件上限不同）返回 `Budget`；超过同步路径预算、
    /// 升完最坏终态容量超出状态预算，由写库前的完整校验拒绝，也是 `Budget`，库保持原样。
    pub fn raise_capacity(&mut self, target: Limits) -> Result<Option<usize>> {
        let current = self.read()?.limits;
        if current == target {
            return Ok(None);
        }
        if !frozen_limits_fit(current, target) {
            return Err(BridgeError::Budget);
        }
        self.edit(Edit::Full, None, |s| {
            let previous = s.limits.max_paths;
            s.limits = target;
            Ok(Some(previous))
        })
    }
}
