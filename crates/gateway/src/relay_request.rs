//! Strict protocol admission prevents hosted tools and caller-selected external targets.
use crate::relay_types::{RelayError, RelayGrant, RelayProvider};
use serde_json::{Value, json};

pub(crate) fn normalize(grant: &RelayGrant, body: &[u8]) -> Result<Vec<u8>, RelayError> {
    if body.len() > grant.limits.request_bytes as usize {
        return Err(RelayError::Request);
    }
    let mut value: Value = serde_json::from_slice(body).map_err(|_| RelayError::Request)?;
    let object = value.as_object_mut().ok_or(RelayError::Request)?;
    if object.get("model").and_then(Value::as_str) != Some(grant.model.as_str()) {
        return Err(RelayError::Denied);
    }
    if object.get("background").and_then(Value::as_bool) == Some(true)
        || object.contains_key("previous_response_id")
        || object.contains_key("conversation")
    {
        return Err(RelayError::Request);
    }
    let token_key = match grant.provider {
        RelayProvider::Anthropic => "max_tokens",
        RelayProvider::OpenAi => "max_output_tokens",
    };
    if object.get(token_key).is_some_and(|v| {
        v.as_u64()
            .is_none_or(|n| n == 0 || n > u64::from(grant.limits.response_tokens))
    }) {
        return Err(RelayError::Request);
    }
    object.insert(token_key.into(), json!(grant.limits.response_tokens));
    object.insert("stream".into(), json!(true));
    if grant.provider == RelayProvider::OpenAi {
        object.insert("store".into(), json!(false));
    }
    inspect(&value, 0)?;
    serde_json::to_vec(&value).map_err(|_| RelayError::Request)
}
fn inspect(value: &Value, depth: u32) -> Result<(), RelayError> {
    if depth > 64 {
        return Err(RelayError::Request);
    }
    match value {
        Value::Array(values) => {
            for item in values {
                inspect(item, depth + 1)?;
            }
        }
        Value::Object(values) => {
            if values.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "image_url" | "file_url" | "file_id" | "server_url" | "base_url"
                )
            }) {
                return Err(RelayError::Request);
            }
            if let Some(kind) = values.get("type").and_then(Value::as_str)
                && matches!(
                    kind,
                    "web_search"
                        | "web_search_preview"
                        | "computer_use_preview"
                        | "mcp"
                        | "file_search"
                        | "code_interpreter"
                        | "image_generation"
                )
            {
                return Err(RelayError::Denied);
            }
            if values.get("name").and_then(Value::as_str).is_some_and(|n| {
                matches!(
                    n,
                    "Agent" | "Task" | "spawn_agent" | "send_input" | "WebFetch" | "WebSearch"
                )
            }) {
                return Err(RelayError::Denied);
            }
            for item in values.values() {
                inspect(item, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
