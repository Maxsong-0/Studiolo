//! 有界的结构化黑板输入。这些 schema 中不能提供任何产物正文。
use crate::{TaskEngineError, TaskSpec, TaskView};
use serde::{Deserialize, Serialize};
use studiolo_control_db::{AttemptProgress, AttemptRecord, BoardRecord};
use studiolo_core::VaultPath;

/// 必填的工作项契约；策略哈希刻意不出现在 AI 输入中。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardTaskSpec {
    #[doc = "人写的任务定义（`TaskSpec`），不含运行时投影。"]
    pub task: TaskSpec,
    #[doc = "有序、可逐条验证的验收标准。"]
    pub acceptance_criteria: Vec<String>,
    #[doc = "独占的逻辑路径（含目录前缀）；空列表表示只读。"]
    pub files_owned: Vec<VaultPath>,
    #[serde(default = "default_max_children")]
    #[doc = "直接子项数上限，最多 8 个。"]
    pub max_children: u8,
    #[doc = "项目内可选的互斥产物目标。"]
    pub artifact_target: Option<String>,
}
const fn default_max_children() -> u8 {
    8
}

/// 内容面引用，绝不含内容字节。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPointer {
    #[doc = "内容面逻辑路径。"]
    pub path: VaultPath,
    #[doc = "精确的不可变或规范源修订号。"]
    pub revision: String,
    #[doc = "SHA-256 内容摘要，十六进制编码。"]
    pub sha256: String,
}

/// 交接必需的不可变文件输入。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileRevision {
    #[doc = "内容面逻辑路径。"]
    pub path: VaultPath,
    #[doc = "精确的不可变或规范源修订号。"]
    pub revision: String,
}

/// 固定到受信任生效策略版本的规则身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConstraint {
    #[doc = "适用规则的身份。"]
    pub rule_id: String,
    #[doc = "包含所引用规则的受信任生效策略哈希。"]
    pub version_hash: String,
}

/// 结构化外发交接，以任务冻结的策略哈希为准。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    #[doc = "交给下一个持有者的单个有界目标。"]
    pub goal: String,
    #[doc = "固定版本的规则引用；绝不复制规则正文。"]
    pub constraints: Vec<RuleConstraint>,
    #[doc = "上一个持有者已作出的决定。"]
    pub decided: Vec<String>,
    #[doc = "明确留给接收者的决定。"]
    pub undecided: Vec<String>,
    #[doc = "接收者必须读取的精确文件修订。"]
    pub required_files: Vec<FileRevision>,
    #[doc = "有序、可逐条验证的验收标准。"]
    pub acceptance_criteria: Vec<String>,
    #[doc = "接收者必须回避的明确操作或路径。"]
    pub forbidden: Vec<String>,
}

/// 逐条验收评估，显式保留未验证与失败的工作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentResult {
    #[doc = "该标准通过。"]
    Pass,
    #[doc = "该标准未通过。"]
    Fail,
    #[doc = "该标准尚未验证。"]
    Unverified,
}

/// 一条完整的验收自评，附证据指针。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceAssessment {
    #[doc = "照录任务定义中的验收标准原文。"]
    pub criterion: String,
    #[doc = "上报的通过、失败或未验证结果。"]
    pub result: AssessmentResult,
    #[doc = "已核验的内容面证据指针。"]
    pub evidence: Vec<ArtifactPointer>,
}

/// 结构化回执允许的四种结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    #[doc = "全部验收标准通过。"]
    Completed,
    #[doc = "该 attempt 失败。"]
    Failed,
    #[doc = "持有者按请求停止了工作。"]
    Stopped,
    #[doc = "持有者放弃了未完成的工作。"]
    Abandoned,
}

/// 保留的补丁引用，含未能落地的补丁。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlandedPointer {
    #[doc = "内容面逻辑路径。"]
    pub path: VaultPath,
    #[doc = "SHA-256 内容摘要，十六进制编码。"]
    pub sha256: String,
}

/// 显式收尾决定。经验教训可以为空，但字段不能省略。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Closeout {
    #[doc = "留作收尾分流的经验教训。"]
    pub learned: Vec<String>,
    #[doc = "是否需要新的架构决策记录（ADR）。"]
    pub adr_required: bool,
    #[doc = "是否需要人工修改策略。"]
    pub rule_change_required: bool,
    #[doc = "经验教训与后续记录的去向。"]
    pub destinations: Vec<VaultPath>,
}

/// 与工作同一认领、同一 fence 下提交的结构化回执。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardReceipt {
    #[doc = "上报的任务结果。"]
    pub status: ReceiptStatus,
    #[doc = "已核验的产物引用，不含产物字节。"]
    pub artifacts: Vec<ArtifactPointer>,
    #[doc = "未解决的冲突路径；列表非空时拒绝终态接受。"]
    pub conflicts: Vec<VaultPath>,
    #[doc = "已保留但未落地的补丁。"]
    pub unlanded: Vec<UnlandedPointer>,
    #[doc = "每条任务验收标准对应一条有序评估。"]
    pub acceptance: Vec<AcceptanceAssessment>,
    #[doc = "仍需后继者完成的具体工作。"]
    pub remaining: Vec<String>,
    #[doc = "执行耗时（毫秒）。"]
    pub elapsed_ms: u64,
    #[doc = "已尝试的工具调用次数（单调计数）。"]
    pub tool_calls: u64,
    #[doc = "留作收尾分流的经验教训。"]
    pub learned: Vec<String>,
    #[doc = "必需的显式收尾决定。"]
    pub closeout: Closeout,
}

/// 机器可读的权威数据加当前规则比对；投影由此派生。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardView {
    /// 候选 staging 指针，含被拒绝或取消的提交。
    pub candidate_stages: Vec<studiolo_control_db::RunnerCandidateStage>,
    /// 最新一次不可变候选发布回执，审阅或拒绝后仍保留。
    pub candidate_receipt: Option<studiolo_vault_broker::ArtifactCommitReceipt>,
    #[doc = "人写任务定义与权威运行时投影。"]
    pub task: TaskView,
    #[doc = "黑板约束与结构化交接/回执元数据。"]
    pub board: BoardRecord,
    #[doc = "当前生效策略是否不同于冻结版本。"]
    pub uses_old_policy: bool,
    #[doc = "当前或最近一次 attempt 的持有者。"]
    pub holder: Option<String>,
    #[doc = "当前或最近一次 attempt 的进度计数。"]
    pub progress: Option<AttemptProgress>,
    #[doc = "当前或最近一次 attempt 的元数据；终态正文只在外部 staging。"]
    pub attempt: Option<AttemptRecord>,
}

/// 原子取得的 attempt 及其机器可读工作项。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardClaim {
    #[doc = "被认领工作项的完整黑板视图（人写定义、权威运行时投影与黑板元数据）。"]
    pub task: BoardView,
    #[doc = "本次认领持有的 attempt 元数据；终态正文只在外部 staging。"]
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
