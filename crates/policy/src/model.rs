//! 三类母本的类型化形状：规则母本为 Markdown，skills 与能力集为 `deny_unknown_fields` 的 JSON。
//!
//! 能力集形状按 PRD-v2-10 §22.2；`version` 与 `policy_version_hash` 由发布事务盖章，
//! 不由提交方填写，因此这里的结构体里没有这两个字段。

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use studiolo_core::VaultPath;

use crate::{PolicyError, validate::Problem};

/// 单份母本正文上限（主人提交）。
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// 接入方提案正文上限。
pub const MAX_PROPOSAL_BYTES: usize = 64 * 1024;

/// 策略母本类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyKind {
    /// 规则母本（Markdown）。
    Rules,
    /// skill（JSON：名称、摘要、触发条件索引、Markdown 正文）。
    Skills,
    /// 某个 AI 类型的能力集（JSON）。
    CapabilitySet,
}

impl PolicyKind {
    /// 库内与审计里使用的稳定名。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Skills => "skills",
            Self::CapabilitySet => "capability_set",
        }
    }

    /// 解析稳定名。
    pub fn parse(value: &str) -> Result<Self, PolicyError> {
        match value {
            "rules" => Ok(Self::Rules),
            "skills" => Ok(Self::Skills),
            "capability_set" => Ok(Self::CapabilitySet),
            _ => Err(PolicyError::InvalidKind),
        }
    }
}

impl fmt::Display for PolicyKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 读权限：项目白名单式枚举；未登记者零权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadCaps {
    /// 白名单前缀（逻辑路径，如 `项目/Studiolo`）；供给视图＝部署上限 ∩ 本清单。
    pub project_whitelist: Vec<String>,
    /// 必须为 true：白名单之外不进入计算。
    pub deny_by_default: bool,
}

/// 写权限；M5 前 `board` 恒为 false，`content_direct` 永久为 false。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteCaps {
    /// 黑板写入（M5 才开放）。
    pub board: bool,
    /// 改人写正文一律 base revision ＋ patch。
    pub content_patch_only: bool,
    /// 永久为 false：AI 不整页覆盖人写正文。
    pub content_direct: bool,
}

/// 策略权限；`activate` 永久为 false（AI 无 One-time Code 路径）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyCaps {
    /// 可提 desired 提案，永不自动生效。
    pub propose_desired: bool,
    /// 永久为 false。
    pub activate: bool,
}

/// 硬绑定：触发条件 → 必须叠加的 skill；由策略平面强制，AI 不能不用。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardBinding {
    /// 触发条件（人可读的一句话）。
    pub when: String,
    /// 必须叠加的 skill 名（须是已发布的 skill）。
    pub must_use: String,
}

/// 一个 AI 类型的能力集。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySet {
    /// AI 类型名，必须等于 subject。
    pub ai_type: String,
    /// 读权限。
    pub read: ReadCaps,
    /// 写权限。
    pub write: WriteCaps,
    /// 策略权限。
    pub policy: PolicyCaps,
    /// 硬绑定 skills。
    #[serde(default)]
    pub hard_bound_skills: Vec<HardBinding>,
}

/// 一个 skill：名称、一句摘要、索引式触发条件与 Markdown 正文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    /// skill 名，必须等于 subject。
    pub name: String,
    /// 一句摘要。
    pub summary: String,
    /// 索引式触发条件（候选，AI 自选；硬绑定在能力集里）。
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Markdown 正文。
    pub markdown: String,
}

/// SHA-256 十六进制。
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 类型名只允许小写字母、数字与连字符，1..=32 字符，不以连字符开头（与 supply 一致）。
pub fn validate_type_name(name: &str) -> Result<(), PolicyError> {
    if lowercase_token(name, 32) {
        Ok(())
    } else {
        Err(PolicyError::InvalidSubject)
    }
}

/// skill 名只允许小写字母、数字与连字符，1..=64 字符，不以连字符开头。
pub fn validate_skill_name(name: &str) -> Result<(), PolicyError> {
    if lowercase_token(name, 64) {
        Ok(())
    } else {
        Err(PolicyError::InvalidSubject)
    }
}

/// 规则母本 subject 是单段文件名：以 `.md` 结尾、不含分隔符、NFC、无控制字符、≤128 字符。
pub fn validate_rules_subject(subject: &str) -> Result<(), PolicyError> {
    let valid = subject.chars().count() <= 128
        && subject.len() > 3
        && subject.ends_with(".md")
        && !subject.starts_with('.')
        && !subject.contains('/')
        && VaultPath::parse(subject).is_ok();
    if valid {
        Ok(())
    } else {
        Err(PolicyError::InvalidSubject)
    }
}

fn lowercase_token(name: &str, max: usize) -> bool {
    (1..=max).contains(&name.len())
        && name
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        && !name.starts_with('-')
}

pub(crate) fn validate_subject(kind: PolicyKind, subject: &str) -> Result<(), PolicyError> {
    match kind {
        PolicyKind::Rules => validate_rules_subject(subject),
        PolicyKind::Skills => validate_skill_name(subject),
        PolicyKind::CapabilitySet => validate_type_name(subject),
    }
}

/// 规范化正文：规则母本统一为 LF、拒绝 NUL；skills／能力集解析成类型后重新序列化。
///
/// 返回的字符串就是入库与计算 sha256 的字节。
pub(crate) fn canonicalize(
    kind: PolicyKind,
    subject: &str,
    body: &str,
    limit: usize,
) -> Result<String, Problem> {
    if body.len() > limit {
        return Err(Problem::new("body", "正文超过上限"));
    }
    if body.contains('\0') {
        return Err(Problem::new("body", "正文含 NUL"));
    }
    match kind {
        PolicyKind::Rules => {
            let text = body.replace("\r\n", "\n");
            if text.trim().is_empty() {
                return Err(Problem::new("body", "规则母本不能为空"));
            }
            Ok(text)
        }
        PolicyKind::Skills => {
            let skill: Skill = serde_json::from_str(body).map_err(json_problem)?;
            if skill.name != subject {
                return Err(Problem::new("name", "skill 名必须等于 subject"));
            }
            serde_json::to_string(&skill).map_err(|_| Problem::new("body", "无法序列化"))
        }
        PolicyKind::CapabilitySet => {
            let set: CapabilitySet = serde_json::from_str(body).map_err(json_problem)?;
            if set.ai_type != subject {
                return Err(Problem::new("ai_type", "ai_type 必须等于 subject"));
            }
            serde_json::to_string(&set).map_err(|_| Problem::new("body", "无法序列化"))
        }
    }
}

fn json_problem(error: serde_json::Error) -> Problem {
    Problem::new(
        format!("line {}:{}", error.line(), error.column()),
        format!("JSON 不符合形状：{}", classify(&error)),
    )
}

/// 只回传结构类别，不回显正文片段。
fn classify(error: &serde_json::Error) -> &'static str {
    match error.classify() {
        serde_json::error::Category::Io => "读取失败",
        serde_json::error::Category::Syntax => "语法错误",
        serde_json::error::Category::Data => "字段缺失、多余或类型不符",
        serde_json::error::Category::Eof => "提前结束",
    }
}
