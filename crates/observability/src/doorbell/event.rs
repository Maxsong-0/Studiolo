//! 审计记录到出站事件的白名单映射。纯逻辑，没有 IO。
//!
//! 表里没有的动作一律不出门。同一件事在账本里有两条记录的（黑板请求一条、
//! 运行时审计一条）只认表里写的那一种，按事件编号前缀区分。

use studiolo_audit_ledger::{AuditPhase, AuditRecord};

/// 往外发的事件类型。新增类型等于新增一类外发信息，须先改 ADR-033 的表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// 有人（主人或接入方）提交了一份待审的策略提案。
    ProposalSubmitted,
    /// 一个策略版本生效了。
    PolicyActivated,
    /// 全局吊销：所有接入方的访问都被收回。
    AccessRevoked,
    /// 全局吊销解除。
    AccessReleased,
    /// 黑板上新建了一个任务。
    TaskCreated,
    /// 黑板上新建了一份交接包。
    HandoffCreated,
    /// 一份回执交回来了。
    HandoffReturned,
    /// 一次认领被放弃（租约过期等），任务等人重新认领。
    TaskAbandoned,
}

impl EventKind {
    /// 对外的稳定事件类型名。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::ProposalSubmitted => "proposal.submitted",
            Self::PolicyActivated => "policy.activated",
            Self::AccessRevoked => "access.revoked",
            Self::AccessReleased => "access.released",
            Self::TaskCreated => "task.created",
            Self::HandoffCreated => "handoff.created",
            Self::HandoffReturned => "handoff.returned",
            Self::TaskAbandoned => "task.abandoned",
        }
    }
}

/// 白名单的一行：动作名、事件编号前缀、可选的「载荷里某个字段必须等于某个值」。
struct Rule {
    action: &'static str,
    id_prefix: &'static str,
    field: Option<(&'static str, &'static str)>,
    kind: EventKind,
}

const POLICY: &str = "policy.v1:";
const BOARD: &str = "board:";
const RUNTIME: &str = "runtime:";
const SUCCEEDED: Option<(&str, &str)> = Some(("status", "succeeded"));

const RULES: [Rule; 8] = [
    Rule {
        action: "policy.desired_submitted",
        id_prefix: POLICY,
        field: None,
        kind: EventKind::ProposalSubmitted,
    },
    Rule {
        action: "policy.activated",
        id_prefix: POLICY,
        field: Some(("decision", "allow")),
        kind: EventKind::PolicyActivated,
    },
    Rule {
        action: "agent.global_revoked",
        id_prefix: POLICY,
        field: None,
        kind: EventKind::AccessRevoked,
    },
    Rule {
        action: "agent.global_released",
        id_prefix: POLICY,
        field: None,
        kind: EventKind::AccessReleased,
    },
    Rule {
        action: "task.create",
        id_prefix: BOARD,
        field: SUCCEEDED,
        kind: EventKind::TaskCreated,
    },
    Rule {
        action: "handoff.created",
        id_prefix: BOARD,
        field: SUCCEEDED,
        kind: EventKind::HandoffCreated,
    },
    Rule {
        action: "handoff.returned",
        id_prefix: BOARD,
        field: SUCCEEDED,
        kind: EventKind::HandoffReturned,
    },
    Rule {
        action: "task.abandoned",
        id_prefix: RUNTIME,
        field: None,
        kind: EventKind::TaskAbandoned,
    },
];

/// 一条要出门的事件。字段都是派生出来的，不含账本里的任何正文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoorbellEvent {
    kind: EventKind,
    message_id: String,
    occurred_at: String,
    occurred_at_ms: i64,
    source_offset: u64,
}

impl DoorbellEvent {
    /// 事件类型。
    #[must_use]
    pub fn kind(&self) -> EventKind {
        self.kind
    }

    /// 稳定的消息编号（`webhook-id`）：重发时不变，收件方按它去重。
    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    /// 事情发生的时间，毫秒时间戳；用来判断是否已经过期。
    #[must_use]
    pub fn occurred_at_ms(&self) -> i64 {
        self.occurred_at_ms
    }

    /// 这条事件对应的账本位置；只在本机用，不出门。
    #[must_use]
    pub fn source_offset(&self) -> u64 {
        self.source_offset
    }

    /// 发出去的正文：`type`、`timestamp`、`data`（最多一个 `link`）。
    #[must_use]
    pub fn body(&self, link: Option<&str>) -> Vec<u8> {
        let data = link.map_or_else(
            || serde_json::json!({}),
            |link| serde_json::json!({ "link": link }),
        );
        serde_json::json!({
            "type": self.kind.code(),
            "timestamp": self.occurred_at,
            "data": data,
        })
        .to_string()
        .into_bytes()
    }
}

/// 这条审计记录要不要出门；要的话变成什么事件。
#[must_use]
pub fn classify(record: &AuditRecord) -> Option<DoorbellEvent> {
    let event = record.event();
    if event.phase() != AuditPhase::Outcome {
        return None;
    }
    let payload = event.payload().as_value();
    let rule = RULES.iter().find(|rule| {
        rule.action == event.action()
            && event.event_id().as_str().starts_with(rule.id_prefix)
            && rule
                .field
                .is_none_or(|(name, expected)| payload[name] == expected)
    })?;
    Some(DoorbellEvent {
        kind: rule.kind,
        message_id: message_id(&record.record_hash().to_hex()),
        occurred_at: record.recorded_at().to_string(),
        occurred_at_ms: record.recorded_at().as_millisecond(),
        source_offset: record.offset(),
    })
}

/// 由账本记录哈希派生的不透明编号；不把哈希本身带出门。
fn message_id(record_hash: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"studiolo.doorbell.message.v1\0");
    digest.update(record_hash.as_bytes());
    format!(
        "msg_{}",
        studiolo_core::to_lower_hex(&digest.finalize()[..16])
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiolo_audit_ledger::{
        AuditEvent, AuditEventId, AuditLedger, MemoryAuditLedger, SanitizedPayload,
    };

    fn record(
        id: &str,
        phase: AuditPhase,
        action: &str,
        payload: serde_json::Value,
    ) -> AuditRecord {
        MemoryAuditLedger::new()
            .append(AuditEvent::new(
                AuditEventId::parse(id).unwrap(),
                phase,
                action,
                SanitizedPayload::try_from(payload).unwrap(),
            ))
            .unwrap()
    }

    #[test]
    fn only_whitelisted_outcomes_with_the_right_origin_leave() {
        let ok = serde_json::json!({"request_hash": "h", "status": "succeeded"});
        let created = classify(&record(
            "board:task.create:1",
            AuditPhase::Outcome,
            "task.create",
            ok.clone(),
        ));
        assert_eq!(created.unwrap().kind(), EventKind::TaskCreated);
        // 意图阶段、被拒绝的请求、别的来源前缀、表外动作都不出门。
        assert!(
            classify(&record(
                "board:task.create:1",
                AuditPhase::Intent,
                "task.create",
                ok.clone()
            ))
            .is_none()
        );
        let rejected = serde_json::json!({"request_hash": "h", "status": "rejected"});
        assert!(
            classify(&record(
                "board:task.create:1",
                AuditPhase::Outcome,
                "task.create",
                rejected
            ))
            .is_none()
        );
        assert!(
            classify(&record(
                "runtime:task.create:1",
                AuditPhase::Outcome,
                "task.create",
                ok.clone()
            ))
            .is_none()
        );
        assert!(
            classify(&record(
                "board:task.claim:1",
                AuditPhase::Outcome,
                "task.claim",
                ok
            ))
            .is_none()
        );
        let read = serde_json::json!({"path": "项目/Alpha/a.md"});
        assert!(classify(&record("read-1", AuditPhase::Outcome, "content.read", read)).is_none());
    }

    #[test]
    fn every_rule_maps_to_its_documented_type() {
        let cases = [
            (
                "policy.v1:policy.desired_submitted:1",
                "policy.desired_submitted",
                serde_json::json!({"actor": "agent"}),
                "proposal.submitted",
            ),
            (
                "policy.v1:policy.activated:1",
                "policy.activated",
                serde_json::json!({"decision": "allow"}),
                "policy.activated",
            ),
            (
                "policy.v1:agent.global_revoked:1",
                "agent.global_revoked",
                serde_json::json!({"level": "global"}),
                "access.revoked",
            ),
            (
                "policy.v1:agent.global_released:1",
                "agent.global_released",
                serde_json::json!({}),
                "access.released",
            ),
            (
                "board:handoff.created:1",
                "handoff.created",
                serde_json::json!({"status": "succeeded"}),
                "handoff.created",
            ),
            (
                "board:handoff.returned:1",
                "handoff.returned",
                serde_json::json!({"status": "succeeded"}),
                "handoff.returned",
            ),
            (
                "runtime:abandoned:1",
                "task.abandoned",
                serde_json::json!({"reason": "lease_expired"}),
                "task.abandoned",
            ),
        ];
        for (id, action, payload, expected) in cases {
            let event = classify(&record(id, AuditPhase::Outcome, action, payload)).unwrap();
            assert_eq!(event.kind().code(), expected);
        }
        // 运行时审计里也有一条 handoff.created；只认黑板请求那一条，同一件事不按两次。
        let runtime = serde_json::json!({"task_id": "t", "attempt_id": "a", "fence": 1});
        assert!(
            classify(&record(
                "runtime:handoff:1",
                AuditPhase::Outcome,
                "handoff.created",
                runtime
            ))
            .is_none()
        );
    }

    #[test]
    fn the_body_is_thin_and_the_id_is_stable_and_opaque() {
        let payload = serde_json::json!({"request_hash": "h", "status": "succeeded"});
        let source = record(
            "board:task.create:1",
            AuditPhase::Outcome,
            "task.create",
            payload,
        );
        let event = classify(&source).unwrap();
        assert_eq!(classify(&source).unwrap(), event);
        assert!(event.message_id().starts_with("msg_") && event.message_id().len() == 36);
        assert!(
            !event
                .message_id()
                .contains(&source.record_hash().to_hex()[..16])
        );

        let bare: serde_json::Value = serde_json::from_slice(&event.body(None)).unwrap();
        assert_eq!(bare["type"], "task.created");
        assert_eq!(bare["data"], serde_json::json!({}));
        assert_eq!(bare.as_object().unwrap().len(), 3);
        let linked: serde_json::Value =
            serde_json::from_slice(&event.body(Some("https://studiolo.example/"))).unwrap();
        assert_eq!(
            linked["data"],
            serde_json::json!({"link": "https://studiolo.example/"})
        );
        // 账本里的任何字段都不出门。
        let text = String::from_utf8(event.body(None)).unwrap();
        for leaked in [
            "request_hash",
            "board:",
            "status",
            &source.record_hash().to_hex(),
        ] {
            assert!(!text.contains(leaked), "事件正文带出了 {leaked}");
        }
    }
}
