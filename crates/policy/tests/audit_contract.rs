//! 策略审计事件：动作名稳定、主体字段齐全、Intent/Outcome 同 id、payload 不含正文与秘密字段。

use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, MemoryAuditLedger};
use studiolo_policy::{
    Activation, PolicyKind, RevocationLevel,
    audit::{self, ActivationId, ActivationRequest, Principal},
};

fn activation() -> Activation {
    Activation {
        version: 3,
        kind: PolicyKind::CapabilitySet,
        subject: "claude".into(),
        retired: false,
        body_sha256: "ab".repeat(32),
        effective_hash: "cd".repeat(32),
        previous_effective_hash: "ef".repeat(32),
        expansions: Vec::new(),
        desired_id: Some("01DESIRED".into()),
        rollback_of: None,
        cleared_type_revocation: true,
        activated_by: "owner".into(),
    }
}

#[test]
fn activation_intent_and_outcome_share_one_id_and_carry_no_body() {
    let ledger = MemoryAuditLedger::new();
    let id = ActivationId::new().unwrap();
    let request = ActivationRequest {
        desired_id: Some("01DESIRED"),
        rollback_of: None,
        release_global_revocation: false,
        expected_effective_hash: &"ef".repeat(32),
    };
    ledger
        .append(id.intent(Principal::Owner, &request).unwrap())
        .unwrap();
    ledger
        .append(id.outcome(Principal::Owner, &activation()).unwrap())
        .unwrap();
    let records = ledger
        .query(&AuditFilter::default().with_action(audit::ACTIVATED))
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().event_id(), records[1].event().event_id());
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    let payload = records[1].event().payload().as_value();
    assert_eq!(payload["actor"], "owner");
    assert_eq!(payload["version"], 3);
    assert_eq!(payload["cleared_type_revocation"], true);
    assert!(payload.get("body").is_none());
    assert!(payload["agent_type"].is_null());
    assert!(
        records[0]
            .event()
            .event_id()
            .as_str()
            .starts_with("policy.v1:policy.activated:")
    );

    let denied = audit::activate_denied(
        Principal::Agent {
            agent_type: "claude",
            instance_id: "01INSTANCE",
        },
        &request,
        "reauthentication_required",
    )
    .unwrap();
    let record = ledger.append(denied).unwrap();
    let payload = record.event().payload().as_value();
    assert_eq!(payload["decision"], "deny");
    assert_eq!(payload["reason"], "reauthentication_required");
    assert_eq!(payload["agent_instance"], "01INSTANCE");

    for (level, action) in [
        (RevocationLevel::Instance, audit::INSTANCE_REVOKED),
        (RevocationLevel::Type, audit::TYPE_REVOKED),
        (RevocationLevel::Global, audit::GLOBAL_REVOKED),
    ] {
        let event = audit::revoked(Principal::Owner, level, "target", "reason").unwrap();
        assert_eq!(event.action(), action);
        ledger.append(event).unwrap();
    }
    ledger
        .append(audit::global_released(Principal::Owner).unwrap())
        .unwrap();
    ledger
        .append(audit::type_restored(Principal::Owner, "claude", 3).unwrap())
        .unwrap();
    ledger
        .append(audit::bootstrapped(&activation()).unwrap())
        .unwrap();
    let released = id
        .released_outcome(Principal::Owner, &"cd".repeat(32))
        .unwrap();
    assert_eq!(released.action(), audit::ACTIVATED);
}
