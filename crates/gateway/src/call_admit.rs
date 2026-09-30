//! 同键调用的准入与等待：锁内先判定再动作，同键仍在执行时在 Condvar 上等待首次结果。

use crate::{
    CallId, GatewayError,
    call_table::{CallAdmission, CallSlot, CallTable, RequestIdentity, SlotGuard},
};
use studiolo_core::AttemptId;

/// `admit` 在锁内做出的判定；先判定再动作，避免在 `BTreeMap` 借用存活时修改它。
enum SlotDecision<Response> {
    Vacant,
    Replay(Response),
    Conflict,
    Evicted,
    Wait,
}

impl<Response: Clone> CallTable<Response> {
    pub(crate) fn admit(
        &self,
        key: (AttemptId, CallId),
        identity: RequestIdentity,
    ) -> Result<CallAdmission<'_, Response>, GatewayError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        loop {
            let decision = match state.slots.get(&key) {
                None => SlotDecision::Vacant,
                Some(CallSlot::Done {
                    identity: prior,
                    response,
                    ..
                }) if *prior == identity => SlotDecision::Replay(response.clone()),
                Some(CallSlot::Done { .. }) => SlotDecision::Conflict,
                Some(CallSlot::Evicted(prior)) if *prior == identity => SlotDecision::Evicted,
                Some(CallSlot::Evicted(_)) => SlotDecision::Conflict,
                Some(CallSlot::InFlight) => SlotDecision::Wait,
            };
            match decision {
                SlotDecision::Vacant => {
                    state.slots.insert(key.clone(), CallSlot::InFlight);
                    let guard = SlotGuard::new(self, key, identity);
                    drop(state);
                    return Ok(CallAdmission::Proceed(guard));
                }
                SlotDecision::Replay(response) => return Ok(CallAdmission::Replay(response)),
                SlotDecision::Conflict => return Ok(CallAdmission::Conflict),
                SlotDecision::Evicted => return Ok(CallAdmission::Evicted),
                SlotDecision::Wait => {
                    #[cfg(test)]
                    self.wait_probe.entered();
                    state = self
                        .finished
                        .wait(state)
                        .map_err(|_| GatewayError::StateUnavailable)?;
                }
            }
        }
    }
}
