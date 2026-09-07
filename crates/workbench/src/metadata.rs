use studiolo_vault_index::{FrontmatterValue, IndexedDocument, MarkdownDocument};

use crate::markdown::clean;

/// Ask M0 for semantic values; scan raw top-level keys only for source positions.
pub(crate) fn value<'a>(
    input: &MarkdownDocument,
    parsed: &'a IndexedDocument,
    keys: &[&str],
) -> Option<(&'a FrontmatterValue, u32)> {
    for key in keys {
        if parsed.frontmatter.warnings.iter().any(|warning| {
            warning
                .canonical_key()
                .is_none_or(|canonical| canonical == *key)
        }) {
            continue;
        }
        let Some(value) = parsed.frontmatter.get(key) else {
            continue;
        };
        let entry = parsed
            .frontmatter
            .entries
            .iter()
            .filter(|entry| entry.canonical_key == *key)
            .max_by_key(|entry| u8::from(entry.raw_key.eq_ignore_ascii_case(key)))?;
        let line = input
            .contents()
            .lines()
            .enumerate()
            .skip(1)
            .take_while(|(_, text)| !matches!(text.trim_end_matches('\r'), "---" | "..."))
            .find_map(|(offset, text)| {
                if text.starts_with(char::is_whitespace) {
                    return None;
                }
                let (raw_key, _) = text.split_once(':')?;
                (raw_key.trim().trim_matches(['\'', '"']) == entry.raw_key)
                    .then_some(offset as u32 + 1)
            })?;
        return Some((value, line));
    }
    None
}

pub(crate) fn text(
    input: &MarkdownDocument,
    parsed: &IndexedDocument,
    keys: &[&str],
) -> Option<(String, u32)> {
    match value(input, parsed, keys)? {
        (FrontmatterValue::Text(value) | FrontmatterValue::Number(value), line) => {
            Some((clean(value), line))
        }
        _ => None,
    }
}

pub(crate) fn raw_status(
    input: &MarkdownDocument,
    parsed: &IndexedDocument,
) -> Option<(String, u32)> {
    let (_, line) = value(input, parsed, &["status"])?;
    let entry = parsed
        .frontmatter
        .entries
        .iter()
        .find(|entry| entry.canonical_key == "status")?;
    Some((
        entry.raw_value.trim().trim_matches(['\'', '"']).to_owned(),
        line,
    ))
}
