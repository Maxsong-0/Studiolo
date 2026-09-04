use studiolo_core::{Classification, VaultPath};

use crate::{FrontmatterValue, NormalizedFrontmatter};

/// Display-only provenance inferred from path shape and declared metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceProvenance {
    /// Ordinary human-authored content without an authenticated classification.
    HumanUnclassified,
    /// A generated TaskEngine/dashboard projection.
    StudioloGenerated,
    /// An attempt draft or other agent-authored artifact.
    AgentArtifact,
    /// An externally sourced note whose declaration has not been manifest-verified.
    ExternalDeclared,
}

/// Advisory source facts extracted by the index.
///
/// This type is intentionally distinct from `studiolo_core::DataLabels`. A
/// frontmatter edit can change declarations but cannot grant egress permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProfile {
    /// Inferred display provenance.
    pub provenance: SourceProvenance,
    /// Declared logical source, if any.
    pub source: Option<String>,
    /// Declared source kind, if any.
    pub source_type: Option<String>,
    /// Untrusted frontmatter classification, if recognized.
    pub declared_classification: Option<Classification>,
    /// Safe classification in the absence of verified policy/manifest metadata.
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
