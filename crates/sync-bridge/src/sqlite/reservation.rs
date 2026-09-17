//! 预留内部重算完整计划并持久游标；不信任调用方 Decision。
use super::{BridgeError, BridgeState, input_codec, lookup::view, model::*, receipts};
use crate::{Decision, Input};

impl BridgeState {
    /// 内部重算整批计划和最坏终态容量，再原子保存原键或 sticky fuse。
    pub fn reserve(&mut self, expected_version: u64, input: Input) -> Result<Reservation> {
        input_codec::encode(&input)?;
        self.edit(Edit::Full, Some(expected_version), |s| {
            if input.scope.binding != s.binding {
                return Err(BridgeError::BindingMismatch);
            }
            if s.fuse.is_some() {
                return Err(BridgeError::Fused);
            }
            if s.pending.is_some() {
                return Err(BridgeError::PendingUnresolved);
            }
            if input.baseline != s.baseline || input.cursor != s.cursor || input.limits != s.limits
            {
                return Err(BridgeError::VersionConflict);
            }
            // 不接受调用方提供 Decision，也不信任持久化的“已规划”布尔标记。
            let decision =
                crate::plan(input.clone(), None).map_err(|_| BridgeError::RetryMismatch)?;
            s.cursor.last_plan_sequence = input.plan_sequence;
            s.cursor.last_canonical_observation = input.canonical.sequence;
            s.cursor.last_mirror_observation = input.mirror.sequence;
            match decision {
                Decision::Ready(plan) => {
                    let cached_hash = receipts::input_hash(&input)?;
                    let actions = (0..plan.actions().len())
                        .map(|i| {
                            Ok(Progress {
                                key: receipts::external_from_hash(&input, cached_hash, i),
                                issued: false,
                                receipt: None,
                                intent_ack: None,
                                outcome_ack: None,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let sequence = input.plan_sequence;
                    s.pending = Some(Pending::new(input, plan, actions, None)?);
                    Ok(Reservation::Ready { sequence })
                }
                Decision::FuseBlocked(_) => {
                    let sequence = input.plan_sequence;
                    s.fuse = Some(Fuse { input, ack: None });
                    Ok(Reservation::Fused { sequence })
                }
            }
        })
    }
    /// 重开取原 Input；没有调用 fresh plan 的许可，不能用它替换执行前的新检查。
    pub fn frozen_pending(&self) -> Result<Option<Input>> {
        Ok(self.read()?.pending.as_ref().map(|p| p.input.clone()))
    }
    /// 仅接受逐字段相同的冻结原输入；不生成新计划或隐含成功。
    pub fn retry_pending(&self, original: &Input) -> Result<Vec<ProgressView>> {
        let s = self.read()?;
        if original.scope.binding != s.binding {
            return Err(BridgeError::BindingMismatch);
        }
        if original.plan_sequence <= s.floor {
            return Err(BridgeError::Retired);
        }
        let p = s.pending.as_ref().ok_or(BridgeError::NotFound)?;
        if &p.input != original {
            return Err(BridgeError::RetryMismatch);
        }
        let plan = receipts::plan(p)?;
        plan.actions()
            .iter()
            .enumerate()
            .map(|(i, a)| view(p, i, a.clone()))
            .collect()
    }
}
