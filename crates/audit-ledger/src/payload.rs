//! 先迭代检查深度，再允许递归 JSON 编码；拒绝时也迭代释放不可信深层值。

use crate::AuditError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_DEPTH: usize = 64;

/// 已检查秘密字段与深度的 JSON 摘要；根深度为 0，任一节点深度不得超过 64。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SanitizedPayload(Value);

impl SanitizedPayload {
    /// 返回通过检查的 JSON 值。
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    pub(crate) fn validate(&self) -> Result<(), AuditError> {
        validate(&self.0)
    }
}

impl TryFrom<Value> for SanitizedPayload {
    type Error = AuditError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        if let Err(error) = validate(&value) {
            discard(value);
            return Err(error);
        }
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for SanitizedPayload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_from(Value::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn validate(value: &Value) -> Result<(), AuditError> {
    let mut pending = vec![(value, 0, String::new(), None)];
    while let Some((value, depth, parent, field)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(AuditError::PayloadTooDeep);
        }
        if field.is_some_and(is_secret_field) {
            return Err(AuditError::SecretField { path: parent });
        }
        match value {
            Value::Object(object) => {
                for (key, nested) in object.iter().rev() {
                    let path = if parent.is_empty() {
                        key.clone()
                    } else {
                        format!("{parent}.{key}")
                    };
                    pending.push((nested, depth + 1, path, Some(key.as_str())));
                }
            }
            Value::Array(array) => {
                for (index, nested) in array.iter().enumerate().rev() {
                    pending.push((nested, depth + 1, format!("{parent}[{index}]"), None));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    Ok(())
}

fn discard(value: Value) {
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(values) => pending.extend(values),
            Value::Object(values) => pending.extend(values.into_iter().map(|(_, value)| value)),
            _ => {}
        }
    }
}

/// 把字段名切成小写词：`-`、`_`、`.`、空白与驼峰边界都算词界，
/// 让 `apiKey`、`x-api-key`、`API_KEY` 归一到同一组词。
fn key_words(key: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_was_lower = false;
    for ch in key.chars() {
        if matches!(ch, '-' | '_' | '.') || ch.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_was_lower = false;
            continue;
        }
        if ch.is_uppercase() && previous_was_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        current.extend(ch.to_lowercase());
        previous_was_lower = ch.is_lowercase() || ch.is_ascii_digit();
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// 单独一个词就足以判定为凭据的词汇；只看字段名的最后一个词，
/// 因此 `token_count`、`session_id` 这类计数与标识不会被误伤。
const SECRET_LAST_WORDS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "cookie",
    "authorization",
    "bearer",
    "credential",
    "credentials",
    "apikey",
    "privatekey",
    "secretkey",
    "accesskey",
    "otp",
    "totp",
    "seed",
];

/// 必须两个词连用才构成凭据的词组（`key`、`code` 单独出现不算）。
const SECRET_LAST_PAIRS: &[(&str, &str)] = &[
    ("api", "key"),
    ("private", "key"),
    ("secret", "key"),
    ("access", "key"),
    ("signing", "key"),
    ("session", "key"),
    ("recovery", "code"),
    ("recovery", "codes"),
];

/// 跟在凭据词后面表示「凭据本体」的后缀：`seed_hex`、`token_value`、`secret_material` 都是
/// 秘密本身，而 `token_count`、`seed_ref`、`session_id` 这类计数/引用/标识不在其中。
const SECRET_VALUE_SUFFIXES: &[&str] = &[
    "hex",
    "value",
    "material",
    "plain",
    "plaintext",
    "raw",
    "b64",
    "base64",
    "bytes",
    "text",
    "str",
    "string",
];

/// 去掉常见英文复数：`tokens`→`token`、`cookies`→`cookie`、`api_keys`→`api_key`。
fn singular(word: &str) -> &str {
    word.strip_suffix("es")
        .filter(|stem| {
            stem.ends_with('s')
                || stem.ends_with('x')
                || stem.ends_with("sh")
                || stem.ends_with("ch")
        })
        .or_else(|| word.strip_suffix('s'))
        .unwrap_or(word)
}

/// 判定切词后的最后一个词（含复数形态）是否与其前一个词构成凭据。
fn ends_with_secret(words: &[String]) -> bool {
    let Some(last) = words.last() else {
        return false;
    };
    let candidates = [last.as_str(), singular(last)];
    if candidates
        .iter()
        .any(|word| SECRET_LAST_WORDS.contains(word))
    {
        return true;
    }
    let Some(previous) = words.len().checked_sub(2).map(|index| &words[index]) else {
        return false;
    };
    SECRET_LAST_PAIRS
        .iter()
        .any(|(first, second)| previous == first && candidates.iter().any(|word| word == second))
}

fn is_secret_field(key: &str) -> bool {
    let words = key_words(key);
    if ends_with_secret(&words) {
        return true;
    }
    // `seed_hex` / `api_key_value`：末词是凭据本体后缀，且去掉它之后仍以凭据词结尾。
    match words.last() {
        Some(last) if SECRET_VALUE_SUFFIXES.contains(&last.as_str()) => {
            ends_with_secret(&words[..words.len() - 1])
        }
        _ => false,
    }
}
