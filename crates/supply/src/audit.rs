//! 接入方审计事件构造：动作名与「四附」矩阵一致，payload 只含类型、实例、路径、revision、
//! tool、token 计数、原因、父实例、登记走的路；不含正文、token 秘密、会话号或宿主路径。事件 ID 每次唯一。

use serde_json::json;
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};

/// 冷启动路牌已交付。
pub const HANDSHAKE_SERVED: &str = "handshake.served";
/// 路牌超硬预算，握手失败。
pub const HANDSHAKE_BUDGET_EXCEEDED: &str = "handshake.budget_exceeded";
/// 审计不可用时的降级握手（由外壳在恢复后补记）。
pub const HANDSHAKE_DEGRADED: &str = "handshake.degraded";
/// 一次内容供给。
pub const CONTENT_READ: &str = "content.read";
/// 被拒绝的 AI 动作。
pub const ACTION_DENIED: &str = "agent.action_denied";
/// 实例自助注册。
pub const INSTANCE_REGISTERED: &str = "agent_instance.registered";

/// 事件主体：类型与实例；未认证时两者皆无。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actor<'a> {
    /// 类型名。
    pub agent_type: Option<&'a str>,
    /// 实例 id。
    pub instance_id: Option<&'a str>,
}

impl<'a> Actor<'a> {
    /// 已认证并注册的实例。
    #[must_use]
    pub fn instance(agent_type: &'a str, instance_id: &'a str) -> Self {
        Self {
            agent_type: Some(agent_type),
            instance_id: Some(instance_id),
        }
    }

    /// 未认证的调用方。
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            agent_type: None,
            instance_id: None,
        }
    }
}

fn event(
    actor: Actor<'_>,
    action: &str,
    mut payload: serde_json::Map<String, serde_json::Value>,
) -> Result<AuditEvent, AuditError> {
    payload.insert(
        "agent_type".into(),
        actor
            .agent_type
            .map_or(serde_json::Value::Null, |value| value.into()),
    );
    payload.insert(
        "agent_instance".into(),
        actor
            .instance_id
            .map_or(serde_json::Value::Null, |value| value.into()),
    );
    let id = AuditEventId::parse(format!(
        "agent.v1:{action}:{}:{}",
        actor.instance_id.unwrap_or("anonymous"),
        ulid::Ulid::generate()
    ))?;
    let payload = SanitizedPayload::try_from(serde_json::Value::Object(payload))?;
    Ok(AuditEvent::new(id, AuditPhase::Outcome, action, payload))
}

fn map(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

/// `handshake.served`：路牌版本＝快照代次，含实际 token 数；接入策略平面时附 effective 哈希，
/// 使每一次握手都能与 `policy.activated` 链对账（PRD-v2-01 2.4 第 7 条）。
pub fn handshake_served(
    actor: Actor<'_>,
    generation: u64,
    token_count: usize,
    effective_hash: Option<&str>,
) -> Result<AuditEvent, AuditError> {
    event(
        actor,
        HANDSHAKE_SERVED,
        map(json!({
            "generation": generation,
            "token_count": token_count,
            "policy_plane": if effective_hash.is_some() { "present" } else { "absent" },
            "effective_hash": effective_hash,
        })),
    )
}

/// `handshake.budget_exceeded`。
pub fn handshake_budget_exceeded(
    actor: Actor<'_>,
    token_count: usize,
    limit: usize,
) -> Result<AuditEvent, AuditError> {
    event(
        actor,
        HANDSHAKE_BUDGET_EXCEEDED,
        map(json!({"token_count": token_count, "limit": limit})),
    )
}

/// `handshake.degraded`：只记 outage id。
pub fn handshake_degraded(actor: Actor<'_>, outage_id: &str) -> Result<AuditEvent, AuditError> {
    event(
        actor,
        HANDSHAKE_DEGRADED,
        map(json!({"outage_id": outage_id})),
    )
}

/// `content.read`：路径、revision 与 tool；不记正文。
pub fn content_read(
    actor: Actor<'_>,
    path: &str,
    revision: &str,
    tool: &str,
) -> Result<AuditEvent, AuditError> {
    event(
        actor,
        CONTENT_READ,
        map(json!({"path": path, "revision": revision, "tool": tool})),
    )
}

/// `agent.action_denied`：动作与结构化原因。
pub fn action_denied(
    actor: Actor<'_>,
    action: &str,
    reason: &str,
) -> Result<AuditEvent, AuditError> {
    event(
        actor,
        ACTION_DENIED,
        map(json!({"action": action, "reason": reason})),
    )
}

/// 没有协商的两条路在两格里都如实写走的哪条路，不留空冒充。
const NOT_NEGOTIATED_HTTP_TEXT: &str = "not_negotiated:http_text";
const NOT_NEGOTIATED_CONNECTOR: &str = "not_negotiated:connector";

/// 实例从哪条路登记，连同 MCP 握手时服务器应答的版本和客户端自报名。
///
/// 内容只有本模块读得到、只进登记审计：自报名没人验过，授权判断不许读它。
#[derive(Clone, Copy, Debug)]
pub struct Registration<'a>(Route<'a>);

#[derive(Clone, Copy, Debug)]
enum Route<'a> {
    Mcp {
        answered: &'a str,
        name: &'a str,
        version: &'a str,
    },
    HttpText,
    Connector,
}

impl<'a> Registration<'a> {
    /// MCP 握手：服务器实际应答的协议版本（不是客户端要的），和客户端自报的名字、版本；
    /// 自报的两样由调用方先限长、去控制字符。
    #[must_use]
    pub fn mcp(answered: &'a str, name: &'a str, version: &'a str) -> Self {
        Self(Route::Mcp {
            answered,
            name,
            version,
        })
    }

    /// HTTP 文本兜底：没有协商，也没有自报名。
    #[must_use]
    pub fn http_text() -> Self {
        Self(Route::HttpText)
    }
}

/// `agent_instance.registered`：直接的父实例（没有就是 null）、协商版本与客户端自报名。MCP 的自报名是对象，
/// 另外两条路是固定字符串，客户端自报什么名字都装不成没有协商的路。
pub fn instance_registered(
    actor: Actor<'_>,
    parent: Option<&str>,
    registration: Registration<'_>,
) -> Result<AuditEvent, AuditError> {
    let (negotiated_version, client_reported) = match registration.0 {
        Route::Mcp {
            answered,
            name,
            version,
        } => (answered, json!({"name": name, "version": version})),
        Route::HttpText => (NOT_NEGOTIATED_HTTP_TEXT, json!(NOT_NEGOTIATED_HTTP_TEXT)),
        Route::Connector => (NOT_NEGOTIATED_CONNECTOR, json!(NOT_NEGOTIATED_CONNECTOR)),
    };
    event(
        actor,
        INSTANCE_REGISTERED,
        map(json!({
            "parent": parent,
            "negotiated_version": negotiated_version,
            "client_reported": client_reported,
        })),
    )
}

/// 连接器的登记。单列一个函数：连接器入口只够得着它（架构守卫按名列着），写不出「协商过」的登记。
pub fn connector_registered(actor: Actor<'_>) -> Result<AuditEvent, AuditError> {
    instance_registered(actor, None, Registration(Route::Connector))
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};

    #[test]
    fn events_carry_actor_and_stable_actions_without_bodies() {
        let ledger = MemoryAuditLedger::new();
        let actor = Actor::instance("claude", "01JT3");
        ledger
            .append(handshake_served(actor, 7, 1234, None).unwrap())
            .unwrap();
        ledger
            .append(
                content_read(actor, "项目/Studiolo/00-总览/INDEX.md", "ab", "read_note").unwrap(),
            )
            .unwrap();
        ledger
            .append(action_denied(Actor::anonymous(), "read_note", "unauthenticated").unwrap())
            .unwrap();
        let reads = ledger
            .query(&AuditFilter::default().with_action(CONTENT_READ))
            .unwrap();
        assert_eq!(reads.len(), 1);
        let payload = reads[0].event().payload().as_value();
        assert_eq!(payload["agent_type"], "claude");
        assert_eq!(payload["agent_instance"], "01JT3");
        assert_eq!(payload["path"], "项目/Studiolo/00-总览/INDEX.md");
        assert!(payload.get("content").is_none());
        assert!(
            reads[0]
                .event()
                .event_id()
                .as_str()
                .starts_with("agent.v1:content.read:01JT3:")
        );
        let denied = ledger
            .query(&AuditFilter::default().with_action(ACTION_DENIED))
            .unwrap();
        assert!(denied[0].event().payload().as_value()["agent_type"].is_null());
        assert!(
            denied[0]
                .event()
                .event_id()
                .as_str()
                .contains(":anonymous:")
        );
        // 两次同类事件 ID 不同，不会撞幂等键。
        let a = handshake_served(actor, 7, 1, Some("ab")).unwrap();
        let b = handshake_served(actor, 7, 1, Some("ab")).unwrap();
        assert_ne!(a.event_id(), b.event_id());
    }

    #[test]
    fn a_registration_says_which_route_it_took_and_never_leaves_the_two_fields_empty() {
        let actor = Actor::instance("claude", "01JT3");
        let payload = |event: AuditEvent| event.payload().as_value().clone();
        for (event, parent, negotiated_version, client_reported) in [
            (
                instance_registered(
                    actor,
                    Some("01JT2"),
                    Registration::mcp("2025-06-18", "codex", "0.171.0"),
                ),
                json!("01JT2"),
                "2025-06-18",
                json!({"name": "codex", "version": "0.171.0"}),
            ),
            // 自报成别的路也只落在名字里。
            (
                instance_registered(
                    actor,
                    None,
                    Registration::mcp("2025-11-25", NOT_NEGOTIATED_CONNECTOR, ""),
                ),
                json!(null),
                "2025-11-25",
                json!({"name": "not_negotiated:connector", "version": ""}),
            ),
            (
                instance_registered(actor, None, Registration::http_text()),
                json!(null),
                "not_negotiated:http_text",
                json!("not_negotiated:http_text"),
            ),
            (
                connector_registered(actor),
                json!(null),
                "not_negotiated:connector",
                json!("not_negotiated:connector"),
            ),
        ] {
            let event = event.unwrap();
            assert_eq!(event.action(), INSTANCE_REGISTERED);
            assert_eq!(
                payload(event),
                json!({
                    "agent_type": "claude",
                    "agent_instance": "01JT3",
                    "parent": parent,
                    "negotiated_version": negotiated_version,
                    "client_reported": client_reported,
                })
            );
        }
    }
}
