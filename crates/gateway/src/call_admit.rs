//! 同键调用的准入与等待：锁内先判定再动作，同键仍在执行时在 Condvar 上等待首次结果。

use crate::{
    CallId, GatewayError,
    call_table::{CallAdmission, CallSlot, CallTable},
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

impl<Request: PartialEq, Response: Clone> CallTable<Request, Response> {
    pub(crate) fn admit(
        &self,
        key: (AttemptId, CallId),
        request: &Request,
    ) -> Result<CallAdmission<Response>, GatewayError> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        loop {
            let decision = match slots.get(&key) {
                None => SlotDecision::Vacant,
                Some(CallSlot::Done {
                    request: prior,
                    response,
                    ..
                }) if prior == request => SlotDecision::Replay(response.clone()),
                Some(CallSlot::Done { .. }) => SlotDecision::Conflict,
                Some(CallSlot::Evicted(prior)) if prior == request => SlotDecision::Evicted,
                Some(CallSlot::Evicted(_)) => SlotDecision::Conflict,
                Some(CallSlot::InFlight) => SlotDecision::Wait,
            };
            match decision {
                SlotDecision::Vacant => {
                    slots.insert(key, CallSlot::InFlight);
                    return Ok(CallAdmission::Proceed);
                }
                SlotDecision::Replay(response) => return Ok(CallAdmission::Replay(response)),
                SlotDecision::Conflict => return Ok(CallAdmission::Conflict),
                SlotDecision::Evicted => return Ok(CallAdmission::Evicted),
                SlotDecision::Wait => {
                    #[cfg(test)]
                    self.wait_probe.entered();
                    slots = self
                        .finished
                        .wait(slots)
                        .map_err(|_| GatewayError::StateUnavailable)?;
                }
            }
        }
    }
}
