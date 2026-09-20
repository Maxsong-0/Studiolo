//! Counts model-emitted actions before any action reaches the isolated CLI.
use crate::relay_types::{RelayError, RelayGrant, RelayProvider};
use serde_json::Value;
use std::collections::BTreeSet;

pub(crate) fn count(bytes: &[u8]) -> Result<u32, RelayError> {
    let text = std::str::from_utf8(bytes).map_err(|_| RelayError::Upstream)?;
    let mut ids = BTreeSet::new();
    for line in text.lines().filter_map(|line| line.strip_prefix("data:")) {
        if line.trim() == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(line.trim()).map_err(|_| RelayError::Upstream)?;
        let mut pending = vec![&event];
        while let Some(value) = pending.pop() {
            match value {
                Value::Array(array) => pending.extend(array.iter()),
                Value::Object(object) => {
                    let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
                    if matches!(kind, "tool_use" | "function_call" | "custom_tool_call") {
                        let name = object.get("name").and_then(Value::as_str).unwrap_or("");
                        if matches!(
                            name,
                            "Agent"
                                | "Task"
                                | "spawn_agent"
                                | "send_input"
                                | "WebFetch"
                                | "WebSearch"
                        ) {
                            return Err(RelayError::Denied);
                        }
                        let id = object
                            .get("id")
                            .or_else(|| object.get("call_id"))
                            .and_then(Value::as_str)
                            .ok_or(RelayError::Upstream)?;
                        ids.insert(id.to_owned());
                    }
                    pending.extend(object.values());
                }
                _ => {}
            }
        }
    }
    u32::try_from(ids.len()).map_err(|_| RelayError::Budget)
}
pub(crate) fn inject_notice(
    grant: &RelayGrant,
    body: &[u8],
    tool_calls: u32,
) -> Result<Vec<u8>, RelayError> {
    let phase = if tool_calls >= grant.limits.wrapup_tools {
        "wrapup: prepare a concise receipt now"
    } else if tool_calls >= grant.limits.pacing_tools {
        "pacing: keep the remaining work bounded"
    } else {
        return Ok(body.to_vec());
    };
    let notice = format!(
        "\nStudiolo budget notice: {phase}. Hard tool ceiling: {}.",
        grant.limits.max_tools
    );
    let mut body: Value = serde_json::from_slice(body).map_err(|_| RelayError::Request)?;
    let key = match grant.provider {
        RelayProvider::Anthropic => "system",
        RelayProvider::OpenAi => "instructions",
    };
    let object = body.as_object_mut().ok_or(RelayError::Request)?;
    match object.get_mut(key) {
        Some(Value::String(value)) => value.push_str(&notice),
        Some(Value::Array(value)) if grant.provider == RelayProvider::Anthropic => {
            value.push(serde_json::json!({"type":"text","text":notice}))
        }
        None | Some(Value::Null) => {
            object.insert(key.into(), Value::String(notice));
        }
        _ => return Err(RelayError::Request),
    }
    let serialized = serde_json::to_vec(&body).map_err(|_| RelayError::Request)?;
    if serialized.len() > grant.limits.request_bytes as usize {
        return Err(RelayError::Request);
    }
    Ok(serialized)
}
