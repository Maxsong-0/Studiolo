use studiolo_core::{Classification, VaultPath};

use crate::{FrontmatterValue, NormalizedFrontmatter};

/// 仅供展示的来源出处，由路径形态与声明的元数据推断。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceProvenance {
    /// 普通的人工撰写内容，没有经过认证的密级。
    HumanUnclassified,
    /// Studiolo 生成的 TaskEngine／仪表盘投影。
    StudioloGenerated,
    /// 尝试草稿或其他由 agent 撰写的产物。
    AgentArtifact,
    /// 外部来源的笔记，其声明未经清单（manifest）核验。
    ExternalDeclared,
}

/// 索引提取出的参考性来源事实。
///
/// 此类型刻意与 `studiolo_core::DataLabels` 区分：改 frontmatter 能改变声明，
/// 但不能授予外发许可。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProfile {
    /// 推断出的展示用出处。
    pub provenance: SourceProvenance,
    /// 声明的逻辑来源（如有）。
    pub source: Option<String>,
    /// 声明的来源类型（如有）。
    pub source_type: Option<String>,
    /// 不可信的 frontmatter 密级声明，能识别时才有值。
    pub declared_classification: Option<Classification>,
    /// 缺少经核验的策略／清单元数据时采用的安全密级（当前恒为 `Restricted`）。
    pub safe_default: Classification,
}

pub(crate) fn profile(path: &VaultPath, frontmatter: &NormalizedFrontmatter) -> SourceProfile {
    let source = text(frontmatter.get("source"));
    let source_type = text(frontmatter.get("source_type"));
    let declared_classification = text(frontmatter.get("classification"))
        .as_deref()
        .and_then(parse_classification);
    let lower_source_type = source_type.as_deref().map(str::to_lowercase);

    let provenance = if is_agent_artifact(path.as_str())
        || lower_source_type
            .as_deref()
            .is_some_and(|value| matches!(value, "agent" | "agent_artifact" | "runner"))
    {
        SourceProvenance::AgentArtifact
    } else if lower_source_type
        .as_deref()
        .is_some_and(|value| matches!(value, "studiolo" | "system" | "projection" | "generated"))
    {
        SourceProvenance::StudioloGenerated
    } else if path.as_str().starts_with("sources/") || source.is_some() || source_type.is_some() {
        SourceProvenance::ExternalDeclared
    } else {
        SourceProvenance::HumanUnclassified
    };

    SourceProfile {
        provenance,
        source,
        source_type,
        declared_classification,
        safe_default: Classification::Restricted,
    }
}

fn text(value: Option<&FrontmatterValue>) -> Option<String> {
    match value {
        Some(FrontmatterValue::Text(value) | FrontmatterValue::Number(value)) => {
            Some(value.clone())
        }
        Some(
            FrontmatterValue::Null
            | FrontmatterValue::Bool(_)
            | FrontmatterValue::List(_)
            | FrontmatterValue::Mapping(_)
            | FrontmatterValue::Status(_),
        )
        | None => None,
    }
}

fn parse_classification(value: &str) -> Option<Classification> {
    match value.trim().to_lowercase().as_str() {
        "public" | "公开" => Some(Classification::Public),
        "internal" | "内部" => Some(Classification::Internal),
        "confidential" | "机密" => Some(Classification::Confidential),
        "restricted" | "受限" => Some(Classification::Restricted),
        _ => None,
    }
}

fn is_agent_artifact(path: &str) -> bool {
    (path.starts_with("projects/") || path.starts_with("项目/"))
        && (path.contains("/drafts/") || path.contains("/草稿/"))
}
