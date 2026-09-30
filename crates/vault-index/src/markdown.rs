use std::collections::{BTreeMap, BTreeSet};

/// 一个解析出的 Markdown 标题。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Heading {
    /// 标题级别，取值 1 到 6（含两端）。
    pub level: u8,
    /// 标题文本，已去掉可选的结尾 `#` 串。
    pub text: String,
    /// 整篇文档内唯一且非空的展示锚点；冲突时递增后缀，纯标点标题使用 `section`。
    pub anchor: String,
    /// 源文件行号（从 1 起）。
    pub line: u32,
}

/// 文档标题树中的一个节点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadingNode {
    /// 本节点代表的标题。
    pub heading: Heading,
    /// 按 Markdown 级别嵌套在它之下的后续标题。
    pub children: Vec<HeadingNode>,
}

pub(crate) struct MarkdownParts<'a> {
    pub(crate) yaml: Option<&'a str>,
    pub(crate) body: &'a str,
    pub(crate) body_first_line: u32,
    pub(crate) closed: bool,
}

pub(crate) fn split_frontmatter(contents: &str) -> MarkdownParts<'_> {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    let Some(first_newline) = contents.find('\n') else {
        return if contents.trim_end_matches('\r') == "---" {
            MarkdownParts {
                yaml: Some(""),
                body: "",
                body_first_line: 1,
                closed: false,
            }
        } else {
            MarkdownParts {
                yaml: None,
                body: contents,
                body_first_line: 1,
                closed: true,
            }
        };
    };
    if contents[..first_newline].trim_end_matches('\r') != "---" {
        return MarkdownParts {
            yaml: None,
            body: contents,
            body_first_line: 1,
            closed: true,
        };
    }

    let yaml_start = first_newline + 1;
    let mut cursor = yaml_start;
    for line in contents[yaml_start..].split_inclusive('\n') {
        if matches!(line.trim_end_matches(['\r', '\n']), "---" | "...") {
            let body_start = cursor + line.len();
            let body_first_line = contents[..body_start]
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count() as u32
                + 1;
            return MarkdownParts {
                yaml: Some(&contents[yaml_start..cursor]),
                body: &contents[body_start..],
                body_first_line,
                closed: true,
            };
        }
        cursor += line.len();
    }
    MarkdownParts {
        yaml: Some(&contents[yaml_start..]),
        body: "",
        body_first_line: 1,
        closed: false,
    }
}

pub(crate) fn parse_headings(body: &str, first_line: u32) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut next_suffix = BTreeMap::<String, usize>::new();
    let mut used_anchors = BTreeSet::new();
    for (number, line) in crate::markdown_scan::visible_lines(body, first_line) {
        let trimmed = line.trim_start();
        let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
        if !(1..=6).contains(&hashes)
            || !trimmed
                .as_bytes()
                .get(hashes)
                .is_some_and(u8::is_ascii_whitespace)
        {
            continue;
        }
        let text = trimmed[hashes..]
            .trim()
            .trim_end_matches('#')
            .trim()
            .to_owned();
        if text.is_empty() {
            continue;
        }
        let base = anchor_for(&text);
        let count = next_suffix.entry(base.clone()).or_default();
        let anchor = loop {
            let candidate = if *count == 0 {
                base.clone()
            } else {
                format!("{base}-{count}")
            };
            *count += 1;
            if used_anchors.insert(candidate.clone()) {
                break candidate;
            }
        };
        headings.push(Heading {
            level: hashes as u8,
            text,
            anchor,
            line: number,
        });
    }
    headings
}

fn anchor_for(text: &str) -> String {
    let mut anchor = String::new();
    let mut last_dash = false;
    for character in text.to_lowercase().chars() {
        if character.is_alphanumeric() || matches!(character, '_' | '-') {
            anchor.push(character);
            last_dash = false;
        } else if character.is_whitespace() && !anchor.is_empty() && !last_dash {
            anchor.push('-');
            last_dash = true;
        }
    }
    let anchor = anchor.trim_end_matches('-');
    if anchor.is_empty() {
        "section".to_owned()
    } else {
        anchor.to_owned()
    }
}

pub(crate) fn build_heading_tree(headings: &[Heading]) -> Vec<HeadingNode> {
    fn push(nodes: &mut Vec<HeadingNode>, heading: Heading) {
        if let Some(parent) = nodes.last_mut()
            && heading.level > parent.heading.level
        {
            push(&mut parent.children, heading);
            return;
        }
        nodes.push(HeadingNode {
            heading,
            children: Vec::new(),
        });
    }

    let mut roots = Vec::new();
    for heading in headings.iter().cloned() {
        push(&mut roots, heading);
    }
    roots
}
