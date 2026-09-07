//! 冻结的因子决定；只有审计白名单事件可对外，不传递种子引用或绑定摘要。

use super::super::{SessionError, events};
use serde::{Deserialize, Serialize};
use serde_json::json;
use studiolo_audit_ledger::{AuditEvent, AuditPhase};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::session) enum Kind {
    Enroll,
    StepUp,
    Recovered { operation: i64 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::session) struct Operation {
    pub sequence: i64,
    pub kind: Kind,
    pub session_id: Option<String>,
    pub epoch: i64,
    pub seed_ref: String,
    pub seed_digest: String,
    pub step: i64,
    pub verified_at: i64,
    pub deadline: i64,
}

impl Operation {
    pub fn event(&self, store_id: &str, phase: AuditPhase) -> Result<AuditEvent, SessionError> {
        let mut payload = json!({"operation":self.sequence,"session_id":self.session_id,
            "identity_version":self.epoch,"verified_at_ms":self.verified_at,
            "expires_at_ms":self.deadline,"decision":if phase==AuditPhase::Intent {"requested"} else {"allow"},
            "publication":"not_released"});
        if let Kind::Recovered { operation } = self.kind {
            payload["recovery_operation"] = json!(operation);
        }
        events::event(
            &format!("{store_id}:totp:{}", self.sequence),
            phase,
            match self.kind {
                Kind::Enroll => "totp.enrolled",
                Kind::StepUp => "totp.step_up",
                Kind::Recovered { .. } => "totp.recovered",
            },
            payload,
        )
    }

    pub fn encoded(&self) -> Result<String, SessionError> {
        let value = serde_json::to_string(self).map_err(|_| SessionError::CorruptStore)?;
        if value.len() > 2048 {
            return Err(SessionError::CorruptStore);
        }
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::session) struct Pending {
    pub operation: Operation,
    pub intent_receipt: Option<String>,
    pub outcome_receipt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::session) struct Proof {
    pub operation: Operation,
    pub intent_receipt: String,
    pub outcome_receipt: String,
    pub activated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::session) struct Factor {
    pub seed_ref: String,
    pub seed_digest: String,
    pub last_step: i64,
    pub consumption: Operation,
    pub enrollment: Option<Proof>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(kind: Kind) -> Operation {
        Operation {
            sequence: 3,
            kind,
            session_id: None,
            epoch: 1,
            seed_ref: "fixture".into(),
            seed_digest: "a".repeat(64),
            step: 2,
            verified_at: 60_000,
            deadline: 360_000,
        }
    }

    #[test]
    fn old_kind_json_and_audit_events_keep_their_exact_v3_meaning() {
        for (kind, encoded, action) in [
            (Kind::Enroll, "\"enroll\"", "totp.enrolled"),
            (Kind::StepUp, "\"step_up\"", "totp.step_up"),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), encoded);
            let operation = operation(kind);
            for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
                let prior = events::event(
                    "fixture-store:totp:3",
                    phase,
                    action,
                    json!({"operation":3,"session_id":null,"identity_version":1,
                        "verified_at_ms":60_000,"expires_at_ms":360_000,
                        "decision":if phase==AuditPhase::Intent {"requested"} else {"allow"},
                        "publication":"not_released"}),
                )
                .unwrap();
                assert_eq!(operation.event("fixture-store", phase).unwrap(), prior);
            }
        }
    }

    #[test]
    fn recovered_kind_has_its_own_tag_and_explicit_recovery_event_binding() {
        let operation = operation(Kind::Recovered { operation: 7 });
        assert_eq!(
            serde_json::to_value(operation.kind).unwrap(),
            json!({"recovered":{"operation":7}})
        );
        let event = operation
            .event("fixture-store", AuditPhase::Outcome)
            .unwrap();
        assert_eq!(event.action(), "totp.recovered");
        assert_eq!(event.payload().as_value()["recovery_operation"], 7);
        assert_eq!(event.event_id().as_str(), "fixture-store:totp:3");
    }
}
