//! 可复用的单文解析。持久形式不含正文、身份或跨文档解析结果。

use crate::{
    DocumentTitle, FrontmatterValue, Heading, IndexedDocument, LinkResolution, MarkdownDocument,
    NormalizedFrontmatter, OutgoingLink, ParseWarning, TitleSource, WikilinkTarget, frontmatter,
    links, markdown, source,
};
use serde::{Deserialize, Serialize};
use studiolo_core::VaultPath;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParsedDocument {
    pub(crate) frontmatter: NormalizedFrontmatter,
    pub(crate) headings: Vec<Heading>,
    pub(crate) links: Vec<ParsedLink>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParsedLink {
    pub(crate) target: WikilinkTarget,
    pub(crate) line: u32,
    pub(crate) column: u32,
    pub(crate) context: String,
}

impl ParsedDocument {
    pub(crate) fn parse(input: &MarkdownDocument) -> Self {
        let parts = markdown::split_frontmatter(input.contents());
        Self {
            frontmatter: frontmatter::normalize(parts.yaml, parts.closed),
            headings: markdown::parse_headings(parts.body, parts.body_first_line),
            links: links::parse_outgoing(parts.body, parts.body_first_line)
                .into_iter()
                .map(|link| ParsedLink {
                    target: link.target,
                    line: link.line,
                    column: link.column,
                    context: link.context,
                })
                .collect(),
        }
    }

    pub(crate) fn finish(self, path: &VaultPath) -> IndexedDocument {
        IndexedDocument {
            path: path.clone(),
            title: select_title(path, &self.frontmatter, &self.headings),
            heading_tree: markdown::build_heading_tree(&self.headings),
            source_profile: source::profile(path, &self.frontmatter),
            warnings: self
                .frontmatter
                .warnings
                .iter()
                .cloned()
                .map(ParseWarning::Frontmatter)
                .collect(),
            frontmatter: self.frontmatter,
            headings: self.headings,
            outgoing_links: self
                .links
                .into_iter()
                .map(|link| OutgoingLink {
                    target: link.target,
                    line: link.line,
                    column: link.column,
                    context: link.context,
                    resolution: LinkResolution::Dangling,
                })
                .collect(),
        }
    }
}

fn select_title(
    path: &VaultPath,
    frontmatter: &NormalizedFrontmatter,
    headings: &[Heading],
) -> DocumentTitle {
    if let Some(FrontmatterValue::Text(title)) = frontmatter.get("title") {
        return DocumentTitle {
            text: title.clone(),
            source: TitleSource::Frontmatter,
        };
    }
    if let Some(heading) = headings.iter().find(|heading| heading.level == 1) {
        return DocumentTitle {
            text: heading.text.clone(),
            source: TitleSource::FirstHeading,
        };
    }
    let filename = path.as_str().rsplit('/').next().unwrap_or(path.as_str());
    DocumentTitle {
        text: filename.strip_suffix(".md").unwrap_or(filename).to_owned(),
        source: TitleSource::FileName,
    }
}
