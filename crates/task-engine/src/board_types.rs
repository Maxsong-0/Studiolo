//! Bounded structured board inputs. No artifact body can be supplied in these schemas.
use crate::{TaskEngineError, TaskSpec, TaskView};
use serde::{Deserialize, Serialize};
use studiolo_control_db::{AttemptProgress, AttemptRecord, BoardRecord};
use studiolo_core::VaultPath;

/// Required work-item contract; the policy hash is deliberately absent from AI input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardTaskSpec {
    #[doc = "Authored task definition and authoritative runtime projection."]
    pub task: TaskSpec,
    #[doc = "Ordered, individually testable acceptance criteria."]
    pub acceptance_criteria: Vec<String>,
    #[doc = "Exclusive logical paths, including directory prefixes; an empty list is read-only."]
    pub files_owned: Vec<VaultPath>,
    #[serde(default = "default_max_children")]
    #[doc = "Maximum direct children, capped at eight."]
    pub max_children: u8,
    #[doc = "Optional mutually exclusive target within the project."]
    pub artifact_target: Option<String>,
}
const fn default_max_children() -> u8 {
    8
}

/// Content-plane reference, never content bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPointer {
    #[doc = "Logical content-plane path."]
    pub path: VaultPath,
    #[doc = "Exact immutable or canonical source revision."]
    pub revision: String,
    #[doc = "SHA-256 content digest, encoded as hexadecimal."]
    pub sha256: String,
}

/// Required immutable file input for a handoff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRevision {
    #[doc = "Logical content-plane path."]
    pub path: VaultPath,
    #[doc = "Exact immutable or canonical source revision."]
    pub revision: String,
}

/// Rule identity pinned to its trusted effective-policy version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConstraint {
    #[doc = "Identity of the applicable rule."]
    pub rule_id: String,
    #[doc = "Trusted effective policy hash containing the referenced rule."]
    pub version_hash: String,
}

/// Structured outbound handoff, fixed against the task's frozen policy hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    #[doc = "One bounded objective for the next holder."]
    pub goal: String,
    #[doc = "Version-pinned rule references; never rule body copies."]
    pub constraints: Vec<RuleConstraint>,
    #[doc = "Decisions already made by the previous holder."]
    pub decided: Vec<String>,
    #[doc = "Decisions explicitly left to the recipient."]
    pub undecided: Vec<String>,
    #[doc = "Exact file revisions the recipient must read."]
    pub required_files: Vec<FileRevision>,
    #[doc = "Ordered, individually testable acceptance criteria."]
    pub acceptance_criteria: Vec<String>,
    #[doc = "Explicit actions or paths the recipient must avoid."]
    pub forbidden: Vec<String>,
}

/// Per-criterion assessment, preserving unverified and failed work explicitly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentResult {
    #[doc = "The criterion passed."]
    Pass,
    #[doc = "The criterion failed."]
    Fail,
    #[doc = "The criterion has not been verified."]
    Unverified,
}

/// One complete acceptance self-assessment with evidence pointers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceAssessment {
    #[doc = "Verbatim criterion from the task definition."]
    pub criterion: String,
    #[doc = "Reported pass, fail, or unverified outcome."]
    pub result: AssessmentResult,
    #[doc = "Verified content-plane evidence pointers."]
    pub evidence: Vec<ArtifactPointer>,
}

/// The four admissible structured receipt outcomes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    #[doc = "All acceptance criteria passed."]
    Completed,
    #[doc = "The attempt failed."]
    Failed,
    #[doc = "The holder stopped work as requested."]
    Stopped,
    #[doc = "The holder abandoned unfinished work."]
    Abandoned,
}

/// Retained patch reference, including patches that could not land.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlandedPointer {
    #[doc = "Logical content-plane path."]
    pub path: VaultPath,
    #[doc = "SHA-256 content digest, encoded as hexadecimal."]
    pub sha256: String,
}

/// Explicit closing decisions. Empty lessons are allowed; omission is not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Closeout {
    #[doc = "Lessons retained for closeout routing."]
    pub learned: Vec<String>,
    #[doc = "Whether a new architecture decision record is needed."]
    pub adr_required: bool,
    #[doc = "Whether a human policy change is needed."]
    pub rule_change_required: bool,
    #[doc = "Destinations for lessons and follow-up records."]
    pub destinations: Vec<VaultPath>,
}

/// Structured receipt under the same claim and fence as the work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardReceipt {
    #[doc = "Reported task outcome."]
    pub status: ReceiptStatus,
    #[doc = "Verified artifact references without artifact bytes."]
    pub artifacts: Vec<ArtifactPointer>,
    #[doc = "Unresolved conflict paths; a nonempty list blocks terminal acceptance."]
    pub conflicts: Vec<VaultPath>,
    #[doc = "Preserved patches that did not land."]
    pub unlanded: Vec<UnlandedPointer>,
    #[doc = "One ordered assessment per task criterion."]
    pub acceptance: Vec<AcceptanceAssessment>,
    #[doc = "Concrete work still requiring a successor."]
    pub remaining: Vec<String>,
    #[doc = "Elapsed execution time in milliseconds."]
    pub elapsed_ms: u64,
    #[doc = "Monotonic number of attempted tool invocations."]
    pub tool_calls: u64,
    #[doc = "Lessons retained for closeout routing."]
    pub learned: Vec<String>,
    #[doc = "Required explicit closing decisions."]
    pub closeout: Closeout,
}

/// Machine-readable authority plus current-rule comparison; projection is derived from this.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardView {
    /// Candidate staging pointers, including rejected or cancelled submissions.
    pub candidate_stages: Vec<studiolo_control_db::RunnerCandidateStage>,
    /// Latest immutable candidate publication receipt, retained after review or rejection.
    pub candidate_receipt: Option<studiolo_vault_broker::ArtifactCommitReceipt>,
    #[doc = "Authored task definition and authoritative runtime projection."]
    pub task: TaskView,
    #[doc = "Blackboard constraints and structured handoff/receipt metadata."]
    pub board: BoardRecord,
    #[doc = "Whether the currently effective policy differs from the frozen version."]
    pub uses_old_policy: bool,
    #[doc = "Current or most recent attempt holder."]
    pub holder: Option<String>,
    #[doc = "Current or most recent attempt counters."]
    pub progress: Option<AttemptProgress>,
    #[doc = "Current or most recent attempt metadata; terminal bodies are staged externally."]
    pub attempt: Option<AttemptRecord>,
}

/// Atomically acquired attempt and its machine-readable work item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardClaim {
    #[doc = "Authored task definition and authoritative runtime projection."]
    pub task: BoardView,
    #[doc = "Current or most recent attempt metadata; terminal bodies are staged externally."]
    pub attempt: AttemptRecord,
}

pub(super) fn reject(reason: &'static str) -> TaskEngineError {
    TaskEngineError::InvalidTaskSpec(reason)
}
pub(super) fn bounded<T: Serialize>(value: &T) -> Result<(), TaskEngineError> {
    if serde_json::to_vec(value)?.len() > 65_536 {
        return Err(reject("structured board object exceeds 64 KiB"));
    }
    Ok(())
}
pub(super) fn strings(values: &[String], required: bool) -> bool {
    values.len() <= 64
        && (!required || !values.is_empty())
        && values
            .iter()
            .all(|v| !v.trim().is_empty() && v.len() <= 2048)
}
pub(super) fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub(super) fn valid_pointer(pointer: &ArtifactPointer) -> bool {
    !pointer.revision.is_empty() && pointer.revision.len() <= 256 && valid_hash(&pointer.sha256)
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn contains_conflict_markers(bytes: &[u8]) -> bool {
    bytes.split(|byte| *byte == b'\n').any(|line| {
        let content = if line.starts_with(b"+") || line.starts_with(b" ") {
            &line[1..]
        } else {
            line
        };
        content.starts_with(b"<<<<<<< ") || content.starts_with(b">>>>>>> ")
    })
}
