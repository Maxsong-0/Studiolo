//! Authoritative audit seam and an in-memory M0 adapter.
//!
//! Audit entries are append-only, idempotent by `(event_id, phase)`, and linked
//! with SHA-256. Callers can only construct payloads that have passed the
//! recursive secret-field check.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{Mutex, MutexGuard},
};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use studiolo_core::{AttemptId, TaskId, ToolId};

/// Stable identity shared by the intent and outcome of one audited operation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct AuditEventId(String);

impl AuditEventId {
    /// Parses a non-empty identifier without control characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, AuditError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(AuditError::InvalidEventId);
        }
        Ok(Self(value))
    }

    /// Returns the textual identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AuditEventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'de> Deserialize<'de> for AuditEventId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

/// The two durable phases of an operation with possible side effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPhase {
    /// Persisted before the sensitive operation starts.
    Intent,
    /// Persisted after an allow, denial, failure, or indeterminate result.
    Outcome,
}

/// Subject metadata used to filter audit records without inspecting payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSubject {
    task_id: Option<TaskId>,
    attempt_id: Option<AttemptId>,
    tool: Option<ToolId>,
}

impl AuditSubject {
    /// Associates a task with this subject.
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Associates an attempt with this subject.
    #[must_use]
    pub fn with_attempt(mut self, attempt_id: AttemptId) -> Self {
        self.attempt_id = Some(attempt_id);
        self
    }

    /// Associates a logical tool with this subject.
    #[must_use]
    pub fn with_tool(mut self, tool: ToolId) -> Self {
        self.tool = Some(tool);
        self
    }

    /// Returns the task identifier, if present.
    #[must_use]
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// Returns the attempt identifier, if present.
    #[must_use]
    pub fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// Returns the logical tool, if present.
    #[must_use]
    pub fn tool(&self) -> Option<&ToolId> {
        self.tool.as_ref()
    }
}

/// JSON payload that was recursively checked for credential-bearing fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SanitizedPayload(Value);

impl SanitizedPayload {
    /// Returns the checked JSON value.
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }
}

impl TryFrom<Value> for SanitizedPayload {
    type Error = AuditError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        reject_secret_fields(&value, "")?;
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for SanitizedPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// One semantic audit event supplied to [`AuditLedger::append`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    event_id: AuditEventId,
    phase: AuditPhase,
    action: String,
    subject: AuditSubject,
    payload: SanitizedPayload,
}

impl AuditEvent {
    /// Creates an event with an empty subject.
    #[must_use]
    pub fn new(
        event_id: AuditEventId,
        phase: AuditPhase,
        action: impl Into<String>,
        payload: SanitizedPayload,
    ) -> Self {
        Self {
            event_id,
            phase,
            action: action.into(),
            subject: AuditSubject::default(),
            payload,
        }
    }

    /// Replaces the event's filterable subject metadata.
    #[must_use]
    pub fn with_subject(mut self, subject: AuditSubject) -> Self {
        self.subject = subject;
        self
    }

    /// Returns the stable event identifier.
    #[must_use]
    pub fn event_id(&self) -> &AuditEventId {
        &self.event_id
    }

    /// Returns the operation phase.
    #[must_use]
    pub fn phase(&self) -> AuditPhase {
        self.phase
    }

    /// Returns the stable action name.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// Returns the event subject.
    #[must_use]
    pub fn subject(&self) -> &AuditSubject {
        &self.subject
    }

    /// Returns the checked payload.
    #[must_use]
    pub fn payload(&self) -> &SanitizedPayload {
        &self.payload
    }
}

/// A SHA-256 digest used by the append-only integrity chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntegrityHash([u8; 32]);

impl IntegrityHash {
    /// Computes a SHA-256 digest.
    #[must_use]
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Returns the fixed genesis digest for a new ledger.
    #[must_use]
    pub fn genesis() -> Self {
        Self::sha256(b"studiolo-audit-genesis-v1")
    }

    /// Returns the lower-case hexadecimal representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
        }
        output
    }
}

impl fmt::Display for IntegrityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Durable result returned by an append operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    offset: u64,
    recorded_at: Timestamp,
    previous_hash: IntegrityHash,
    record_hash: IntegrityHash,
    event: AuditEvent,
}

impl AuditRecord {
    /// Reconstructs a record from authoritative adapter-owned storage parts.
    ///
    /// The adapter is responsible for validating the offset and integrity hash
    /// before exposing the record through [`AuditLedger`].
    #[must_use]
    pub fn from_parts(
        offset: u64,
        recorded_at: Timestamp,
        previous_hash: IntegrityHash,
        record_hash: IntegrityHash,
        event: AuditEvent,
    ) -> Self {
        Self {
            offset,
            recorded_at,
            previous_hash,
            record_hash,
            event,
        }
    }

    /// Returns the one-based append offset.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the server-side recording timestamp.
    #[must_use]
    pub fn recorded_at(&self) -> Timestamp {
        self.recorded_at
    }

    /// Returns the preceding record hash, or the genesis hash for offset one.
    #[must_use]
    pub fn previous_hash(&self) -> IntegrityHash {
        self.previous_hash
    }

    /// Returns this record's SHA-256 chain digest.
    #[must_use]
    pub fn record_hash(&self) -> IntegrityHash {
        self.record_hash
    }

    /// Returns the appended semantic event.
    #[must_use]
    pub fn event(&self) -> &AuditEvent {
        &self.event
    }
}

/// Public query constraints. Unset fields do not restrict results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditFilter {
    event_id: Option<AuditEventId>,
    task_id: Option<TaskId>,
    attempt_id: Option<AttemptId>,
    tool: Option<ToolId>,
    phase: Option<AuditPhase>,
    action: Option<String>,
    after_offset: Option<u64>,
    limit: Option<usize>,
}

impl AuditFilter {
    /// Filters by event identifier.
    #[must_use]
    pub fn with_event_id(mut self, event_id: AuditEventId) -> Self {
        self.event_id = Some(event_id);
        self
    }

    /// Filters by task identifier.
    #[must_use]
    pub fn with_task(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Filters by attempt identifier.
    #[must_use]
    pub fn with_attempt(mut self, attempt_id: AttemptId) -> Self {
        self.attempt_id = Some(attempt_id);
        self
    }

    /// Filters by tool identifier.
    #[must_use]
    pub fn with_tool(mut self, tool: ToolId) -> Self {
        self.tool = Some(tool);
        self
    }

    /// Filters by phase.
    #[must_use]
    pub fn with_phase(mut self, phase: AuditPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    /// Filters by exact stable action name.
    #[must_use]
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Returns records whose offset is strictly greater than `offset`.
    #[must_use]
    pub fn after_offset(mut self, offset: u64) -> Self {
        self.after_offset = Some(offset);
        self
    }

    /// Limits the number of records returned in append order.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Returns the event identifier constraint.
    #[must_use]
    pub fn event_id(&self) -> Option<&AuditEventId> {
        self.event_id.as_ref()
    }

    /// Returns the task constraint.
    #[must_use]
    pub fn task_id(&self) -> Option<TaskId> {
        self.task_id
    }

    /// Returns the attempt constraint.
    #[must_use]
    pub fn attempt_id(&self) -> Option<AttemptId> {
        self.attempt_id
    }

    /// Returns the logical tool constraint.
    #[must_use]
    pub fn tool(&self) -> Option<&ToolId> {
        self.tool.as_ref()
    }

    /// Returns the phase constraint.
    #[must_use]
    pub fn phase(&self) -> Option<AuditPhase> {
        self.phase
    }

    /// Returns the exact action constraint.
    #[must_use]
    pub fn action(&self) -> Option<&str> {
        self.action.as_deref()
    }

    /// Returns the exclusive lower offset bound.
    #[must_use]
    pub fn offset_exclusive(&self) -> Option<u64> {
        self.after_offset
    }

    /// Returns the maximum number of records requested.
    #[must_use]
    pub fn limit(&self) -> Option<usize> {
        self.limit
    }
}

/// Errors at the authoritative audit seam.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AuditError {
    /// An event identifier was empty or contained control characters.
    #[error("audit event id is invalid")]
    InvalidEventId,
    /// A payload key could carry a secret and therefore cannot be persisted.
    #[error("audit payload contains forbidden secret field at {path}")]
    SecretField {
        /// Dot-and-index path of the rejected field.
        path: String,
    },
    /// An existing idempotency key was reused for different semantic content.
    #[error("audit event {event_id}/{phase:?} conflicts with an existing record")]
    IdempotencyConflict {
        /// Conflicting stable identifier.
        event_id: AuditEventId,
        /// Conflicting phase.
        phase: AuditPhase,
    },
    /// The event could not be encoded for hashing and persistence.
    #[error("audit event serialization failed")]
    Serialization,
    /// The authoritative audit store is unavailable.
    #[error("audit ledger is unavailable")]
    Unavailable,
}

/// Synchronous, object-safe boundary for authoritative audit persistence.
pub trait AuditLedger: Send + Sync {
    /// Appends an event or returns the existing record for an identical retry.
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError>;

    /// Queries records in append order.
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError>;
}

/// In-memory contract adapter used by M0 module and integration tests.
#[derive(Debug, Default)]
pub struct MemoryAuditLedger {
    state: Mutex<MemoryState>,
}

#[derive(Debug, Default)]
struct MemoryState {
    records: Vec<AuditRecord>,
    keys: BTreeMap<(AuditEventId, AuditPhase), usize>,
}

impl MemoryAuditLedger {
    /// Creates an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, MemoryState>, AuditError> {
        self.state.lock().map_err(|_| AuditError::Unavailable)
    }
}

impl AuditLedger for MemoryAuditLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let mut state = self.lock()?;
        let key = (event.event_id.clone(), event.phase);
        if let Some(index) = state.keys.get(&key).copied() {
            let existing = &state.records[index];
            return if existing.event == event {
                Ok(existing.clone())
            } else {
                Err(AuditError::IdempotencyConflict {
                    event_id: key.0,
                    phase: key.1,
                })
            };
        }

        let offset = state.records.len() as u64 + 1;
        let recorded_at = Timestamp::now();
        let previous_hash = state
            .records
            .last()
            .map_or_else(IntegrityHash::genesis, AuditRecord::record_hash);
        let encoded = serde_json::to_vec(&(offset, recorded_at, previous_hash, &event))
            .map_err(|_| AuditError::Serialization)?;
        let domain = b"studiolo-audit-record-v1\0";
        let mut material = Vec::with_capacity(domain.len() + 32 + encoded.len());
        material.extend_from_slice(domain);
        material.extend_from_slice(&previous_hash.0);
        material.extend_from_slice(&encoded);
        let record_hash = IntegrityHash::sha256(&material);
        let record = AuditRecord {
            offset,
            recorded_at,
            previous_hash,
            record_hash,
            event,
        };
        let index = state.records.len();
        state.keys.insert(key, index);
        state.records.push(record.clone());
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let state = self.lock()?;
        let limit = filter.limit.unwrap_or(usize::MAX);
        Ok(state
            .records
            .iter()
            .filter(|record| {
                filter
                    .event_id
                    .as_ref()
                    .is_none_or(|value| value == record.event.event_id())
                    && filter
                        .task_id
                        .is_none_or(|value| Some(value) == record.event.subject().task_id())
                    && filter
                        .attempt_id
                        .is_none_or(|value| Some(value) == record.event.subject().attempt_id())
                    && filter.tool.as_ref().is_none_or(|value| {
                        record
                            .event
                            .subject()
                            .tool()
                            .is_some_and(|tool| tool == value)
                    })
                    && filter
                        .phase
                        .is_none_or(|value| value == record.event.phase())
                    && filter
                        .action
                        .as_deref()
                        .is_none_or(|value| value == record.event.action())
                    && filter
                        .after_offset
                        .is_none_or(|value| record.offset() > value)
            })
            .take(limit)
            .cloned()
            .collect())
    }
}

fn reject_secret_fields(value: &Value, parent: &str) -> Result<(), AuditError> {
    match value {
        Value::Object(object) => {
            for (key, nested) in object {
                let path = if parent.is_empty() {
                    key.clone()
                } else {
                    format!("{parent}.{key}")
                };
                if is_secret_field(key) {
                    return Err(AuditError::SecretField { path });
                }
                reject_secret_fields(nested, &path)?;
            }
        }
        Value::Array(array) => {
            for (index, nested) in array.iter().enumerate() {
                reject_secret_fields(nested, &format!("{parent}[{index}]"))?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
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
