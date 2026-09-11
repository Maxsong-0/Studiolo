//! frontmatter 正规化；仅解释输入文本，不执行文件操作。

use crate::frontmatter_types::*;
use crate::frontmatter_yaml::YamlValue;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn normalize(yaml: Option<&str>, closed: bool) -> NormalizedFrontmatter {
    let Some(yaml) = yaml else {
        return NormalizedFrontmatter::default();
    };
    let mut normalized = NormalizedFrontmatter {
        raw: yaml.to_owned(),
        ..NormalizedFrontmatter::default()
    };
    if !closed {
        normalized.warnings.push(FrontmatterWarning::Unclosed);
        return normalized;
    }
    let parsed = match serde_saphyr::from_str::<BTreeMap<String, YamlValue>>(yaml) {
        Ok(parsed) => parsed,
        Err(error) => {
            normalized.warnings.push(FrontmatterWarning::MalformedYaml {
                message: error.to_string(),
            });
            return normalized;
        }
    };

    let mut selected: BTreeMap<String, (u8, FrontmatterValue)> = BTreeMap::new();
    let mut spellings: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for raw in scan_raw_entries(yaml) {
        let Some(value) = parsed.get(&raw.raw_key) else {
            continue;
        };
        let (canonical_key, priority, alias_hit) = canonical_key(&raw.raw_key);
        let value = normalize_value(&canonical_key, value.clone());
        if alias_hit {
            normalized
                .alias_hits
                .push((raw.raw_key.clone(), canonical_key.clone()));
        }
        spellings
            .entry(canonical_key.clone())
            .or_default()
            .insert(raw.raw_key.clone());
        if selected
            .get(&canonical_key)
            .is_none_or(|(existing, _)| priority > *existing)
        {
            selected.insert(canonical_key.clone(), (priority, value.clone()));
        }
        normalized.entries.push(FrontmatterEntry {
            raw_key: raw.raw_key,
            raw_value: raw.raw_value,
            canonical_key,
            value,
        });
    }
    normalized.canonical = selected
        .into_iter()
        .map(|(key, (_, value))| (key, value))
        .collect();
    for (canonical, keys) in spellings {
        if keys.len() > 1 {
            normalized
                .warnings
                .push(FrontmatterWarning::DuplicateAliasGroup {
                    canonical,
                    raw_keys: keys.into_iter().collect(),
                });
        }
    }
    normalized
}

struct RawEntry {
    raw_key: String,
    raw_value: String,
}

fn scan_raw_entries(yaml: &str) -> Vec<RawEntry> {
    let mut starts = Vec::<(String, usize, usize)>::new();
    let mut line_start = 0;
    for line in yaml.split_inclusive('\n') {
        let logical = line.trim_end_matches(['\r', '\n']);
        if !logical.starts_with(char::is_whitespace)
            && !logical.starts_with('#')
            && let Some(colon) = logical.find(':')
        {
            let key = logical[..colon].trim();
            if !key.is_empty() {
                starts.push((
                    key.trim_matches(['\'', '"']).to_owned(),
                    line_start + colon + 1,
                    line_start,
                ));
            }
        }
        line_start += line.len();
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, (raw_key, value_start, _))| {
            let value_end = starts
                .get(index + 1)
                .map_or(yaml.len(), |(_, _, next_start)| *next_start);
            RawEntry {
                raw_key: raw_key.clone(),
                raw_value: yaml[*value_start..value_end].trim().to_owned(),
            }
        })
        .collect()
}

fn canonical_key(raw: &str) -> (String, u8, bool) {
    for group in FRONTMATTER_KEY_ALIASES
        .iter()
        .chain(FRONTMATTER_SPECIAL_KEY_ALIASES)
    {
        if raw.eq_ignore_ascii_case(group.canonical) {
            return (group.canonical.to_owned(), 2, false);
        }
        if group.aliases.contains(&raw) {
            return (group.canonical.to_owned(), 1, true);
        }
    }
    (raw.to_owned(), 2, false)
}

fn normalize_value(key: &str, value: YamlValue) -> FrontmatterValue {
    if key == "status"
        && let Some(text) = scalar_text(&value)
    {
        return FrontmatterValue::Status(normalize_status(text));
    }
    if key == "type"
        && let Some(text) = scalar_text(&value)
        && is_follow_up_type(text)
    {
        return FrontmatterValue::Text("follow-up".to_owned());
    }
    match value {
        YamlValue::Null => FrontmatterValue::Null,
        YamlValue::Bool(value) => FrontmatterValue::Bool(value),
        YamlValue::Number(value) => FrontmatterValue::Number(value),
        YamlValue::Text(value) => FrontmatterValue::Text(value),
        YamlValue::Sequence(values) => FrontmatterValue::List(
            values
                .into_iter()
                .map(|value| normalize_value("", value))
                .collect(),
        ),
        YamlValue::Mapping(values) => FrontmatterValue::Mapping(
            values
                .into_iter()
                .map(|(key, value)| (key, normalize_value("", value)))
                .collect(),
        ),
    }
}

fn is_follow_up_type(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "follow-up"
            | "followup-suggestion"
            | "followup-suggestions"
            | "follow-up-suggestion"
            | "follow-up-suggestions"
            | "后续建议"
    )
}

fn scalar_text(value: &YamlValue) -> Option<&str> {
    match value {
        YamlValue::Text(value) | YamlValue::Number(value) => Some(value),
        YamlValue::Null | YamlValue::Bool(_) | YamlValue::Sequence(_) | YamlValue::Mapping(_) => {
            None
        }
    }
}

fn normalize_status(raw: &str) -> NormalizedStatus {
    let mut value = raw.trim();
    loop {
        value = value.trim_start();
        if let Some(rest) = value.strip_prefix("**") {
            value = rest;
            continue;
        }
        match value.chars().next() {
            Some(character) if is_emoji_decoration(character) => {
                value = &value[character.len_utf8()..];
            }
            _ => break,
        }
    }
    let split_at = value
        .char_indices()
        .find_map(|(index, character)| "（(·#+".contains(character).then_some(index));
    if let Some(index) = split_at {
        value = &value[..index];
    }
    value = value.trim().trim_end_matches("**").trim();
    let lower = value.to_lowercase();

    if lower.starts_with("done-pending-restart")
        || lower.starts_with("代码完成")
        || ["待执行", "待评审", "待实现", "待用户重启"]
            .iter()
            .any(|prefix| lower.starts_with(prefix))
    {
        NormalizedStatus::Pending
    } else if lower.starts_with("已采纳") {
        NormalizedStatus::Adopted
    } else if lower.starts_with("已修复") {
        NormalizedStatus::Fixed
    } else if ["done", "完成", "已完成", "已上线", "全部完成"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        NormalizedStatus::Done
    } else if lower.starts_with("进行中") {
        NormalizedStatus::InProgress
    } else if lower.starts_with("已批准") || lower.starts_with("设计已批") {
        NormalizedStatus::Approved
    } else if lower.starts_with("已被取代") {
        NormalizedStatus::Superseded
    } else if lower.starts_with("已废弃") {
        NormalizedStatus::Deprecated
    } else if lower.starts_with("草稿") || lower.starts_with("构想") {
        NormalizedStatus::Draft
    } else {
        NormalizedStatus::Unknown(raw.to_owned())
    }
}

fn is_emoji_decoration(character: char) -> bool {
    matches!(
        character,
        '\u{200d}' | '\u{20e3}' | '\u{fe0f}' | '\u{2300}'..='\u{27ff}' | '\u{1f000}'..='\u{1faff}'
    )
}
