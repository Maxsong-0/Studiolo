//! Shared value types and ports used at Studiolo's module seams.

use std::{
    collections::BTreeSet,
    fmt,
    num::NonZeroU64,
    path::{Component, Path},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
pub mod checkpoint;
mod revision_hasher;
pub use revision_hasher::RevisionHasher;
use ulid::Ulid;
use uuid::Uuid;

/// Error returned when a textual identifier is malformed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind}: {value}")]
pub struct IdParseError {
    kind: &'static str,
    value: String,
}

/// Stable identifier for a task.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Ulid);

impl TaskId {
    /// Generates a new task identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for TaskId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ulid::from_string(value)
            .map(Self)
            .map_err(|_| IdParseError {
                kind: "task id",
                value: value.to_owned(),
            })
    }
}

/// Stable identifier for one execution attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AttemptId(Ulid);

impl AttemptId {
    /// Generates a new attempt identifier.
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::generate())
    }
}

impl Default for AttemptId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AttemptId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "A-{}", self.0)
    }
}

impl FromStr for AttemptId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let raw = value.strip_prefix("A-").ok_or_else(|| IdParseError {
            kind: "attempt id",
            value: value.to_owned(),
        })?;
        Ulid::from_string(raw).map(Self).map_err(|_| IdParseError {
            kind: "attempt id",
            value: value.to_owned(),
        })
    }
}

impl TryFrom<String> for AttemptId {
    type Error = IdParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<AttemptId> for String {
    fn from(value: AttemptId) -> Self {
        value.to_string()
    }
}

/// Persistent identity of one canonical Vault.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VaultUuid(Uuid);

impl VaultUuid {
    /// Generates a fresh Vault identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for VaultUuid {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for VaultUuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for VaultUuid {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self).map_err(|_| IdParseError {
            kind: "vault uuid",
            value: value.to_owned(),
        })
    }
}

/// Reason a relative Vault path was rejected before any file-system access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathSyntaxError {
    /// The path was empty.
    #[error("path is empty")]
    Empty,
    /// The path was absolute or contained `.` or `..` components.
    #[error("path is not a clean relative path")]
    NotRelative,
    /// The path contained a backslash, encoded separator, NUL, control character, a
    /// combining mark, or any spelling that is not Unicode NFC.
    #[error("path contains a forbidden encoding or character")]
    ForbiddenEncoding,
}

/// A syntactically safe path relative to the approved Vault root.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct VaultPath(String);

impl VaultPath {
    /// Parses a path without consulting the ambient file system.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, PathSyntaxError> {
        let raw = raw.as_ref();
        if raw.is_empty() {
            return Err(PathSyntaxError::Empty);
        }
        let lower = raw.to_ascii_lowercase();
        let bytes = raw.as_bytes();
        let has_windows_drive_prefix = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/';
        // 只接受 NFC 拼写：分解形式（含日文浊音符、Hangul jamo 等黑名单外的形态）与
        // 组合形式在归一化不敏感的文件系统上指向同一物理文件，会造成两套逻辑身份。
        if raw.contains('\\')
            || raw.chars().any(char::is_control)
            || raw.chars().any(is_unicode_combining_mark)
            || !unicode_normalization::is_nfc(raw)
            || has_windows_drive_prefix
            || lower.contains("%2f")
            || lower.contains("%5c")
            || lower.contains("%00")
            || lower.contains("%2e")
        {
            return Err(PathSyntaxError::ForbiddenEncoding);
        }
        let path = Path::new(raw);
        if path.is_absolute()
            || raw
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::Prefix(_)
                        | Component::RootDir
                        | Component::CurDir
                        | Component::ParentDir
                )
            })
        {
            return Err(PathSyntaxError::NotRelative);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Returns the original relative path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether ordinary callers are forbidden from mutating this path.
    #[must_use]
    pub fn is_protected(&self) -> bool {
        let folded = self.0.to_ascii_lowercase();
        let first = folded.split('/').next().unwrap_or_default();
        matches!(first, ".git" | ".workbench" | ".obsidian") || folded == ".stignore"
    }

    /// Returns a case-folded key used for conservative collision checks. Parsed paths are
    /// already NFC, so folding alone makes keys comparable with NFC-normalized entry names.
    #[must_use]
    pub fn collision_key(&self) -> String {
        self.0.to_lowercase()
    }
}

fn is_unicode_combining_mark(character: char) -> bool {
    matches!(
        character as u32,
        0x0300..=0x036f
            | 0x1ab0..=0x1aff
            | 0x1dc0..=0x1dff
            | 0x20d0..=0x20ff
            | 0xfe20..=0xfe2f
    )
}

impl fmt::Display for VaultPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for VaultPath {
    type Err = PathSyntaxError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for VaultPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

/// Content revision bound to a Vault identity and logical path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision([u8; 32]);

impl Revision {
    /// Computes the SHA-256 revision for content at a path in a Vault.
    #[must_use]
    pub fn compute(vault: VaultUuid, path: &VaultPath, content: &[u8]) -> Self {
        let mut hasher = RevisionHasher::new(vault, path);
        hasher.update(content);
        hasher.finish()
    }

    /// Returns the lower-case hexadecimal digest.
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

impl fmt::Display for Revision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Monotonically increasing token that fences one attempt from older attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fence(NonZeroU64);

impl Fence {
    /// Constructs a non-zero fence.
    #[must_use]
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    /// Returns the numeric fence value.
    #[must_use]
    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// Stable logical tool name such as `web.fetch`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ToolId(String);

impl ToolId {
    /// Constructs a tool identifier from a compile-time constant.
    #[must_use]
    pub fn from_static(value: &'static str) -> Self {
        assert!(
            !value.is_empty() && !value.chars().any(char::is_control),
            "invalid tool identifier"
        );
        Self(value.to_owned())
    }

    /// Returns the logical tool name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ToolId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(serde::de::Error::custom("invalid tool identifier"));
        }
        Ok(Self(value))
    }
}

/// A set of permissions that exposes narrowing, never widening, as its operation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PermissionSet(BTreeSet<ToolId>);

impl PermissionSet {
    /// Constructs a permission set from explicitly granted tools.
    #[must_use]
    pub fn new(tools: impl IntoIterator<Item = ToolId>) -> Self {
        Self(tools.into_iter().collect())
    }

    /// Returns the intersection of two independently imposed limits.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        Self(self.0.intersection(&other.0).cloned().collect())
    }

    /// Returns whether a tool is present in the effective set.
    #[must_use]
    pub fn allows(&self, tool: &ToolId) -> bool {
        self.0.contains(tool)
    }
}

/// Sensitivity of source content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    /// Public data.
    Public,
    /// Internal data.
    Internal,
    /// Confidential data.
    Confidential,
    /// Restricted data with the strongest default deny rules.
    Restricted,
}

/// Amount of source material eligible for durable backup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupScope {
    /// Full validated object may be backed up.
    Full,
    /// Only a redacted summary and manifest may be backed up.
    SummaryOnly,
    /// Only a non-content tombstone may be backed up.
    None,
}

/// Amount of source material eligible for desktop synchronization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSync {
    /// Nothing is synchronized.
    Deny,
    /// Only a redacted summary is synchronized.
    SummaryOnly,
}

/// Whether source material may be sent to a model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEgress {
    /// Model egress is forbidden.
    Deny,
    /// Only an approved local model may consume it.
    LocalOnly,
    /// Only the named providers may consume it.
    AllowProviders(BTreeSet<String>),
}

/// Error returned for a contradictory source-label combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DataLabelError {
    /// Content excluded from backup cannot be synchronized to a desktop.
    #[error("backup_scope=none requires desktop_sync=deny")]
    DesktopOutlivesBackup,
    /// Restricted content cannot be approved for a cloud provider.
    #[error("restricted content cannot allow a cloud model provider")]
    RestrictedProviderEgress,
    /// A provider allow-list must name at least one provider.
    #[error("model provider allow-list is empty")]
    EmptyProviderList,
}

/// Validated classification, retention, and egress labels inherited by an object.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DataLabels {
    classification: Classification,
    backup_scope: BackupScope,
    desktop_sync: DesktopSync,
    model_egress: ModelEgress,
}

impl<'de> Deserialize<'de> for DataLabels {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            classification: Classification,
            backup_scope: BackupScope,
            desktop_sync: DesktopSync,
            model_egress: ModelEgress,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::new(
            fields.classification,
            fields.backup_scope,
            fields.desktop_sync,
            fields.model_egress,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl DataLabels {
    /// Validates and constructs an inseparable label bundle.
    pub fn new(
        classification: Classification,
        backup_scope: BackupScope,
        desktop_sync: DesktopSync,
        model_egress: ModelEgress,
    ) -> Result<Self, DataLabelError> {
        if backup_scope == BackupScope::None && desktop_sync != DesktopSync::Deny {
            return Err(DataLabelError::DesktopOutlivesBackup);
        }
        if matches!(&model_egress, ModelEgress::AllowProviders(providers) if providers.is_empty()) {
            return Err(DataLabelError::EmptyProviderList);
        }
        if classification == Classification::Restricted
            && matches!(model_egress, ModelEgress::AllowProviders(_))
        {
            return Err(DataLabelError::RestrictedProviderEgress);
        }
        Ok(Self {
            classification,
            backup_scope,
            desktop_sync,
            model_egress,
        })
    }

    /// Returns the classification.
    #[must_use]
    pub fn classification(&self) -> Classification {
        self.classification
    }

    /// Returns the backup scope.
    #[must_use]
    pub fn backup_scope(&self) -> BackupScope {
        self.backup_scope
    }

    /// Returns the desktop synchronization scope.
    #[must_use]
    pub fn desktop_sync(&self) -> DesktopSync {
        self.desktop_sync
    }

    /// Returns the model egress rule.
    #[must_use]
    pub fn model_egress(&self) -> &ModelEgress {
        &self.model_egress
    }
}
