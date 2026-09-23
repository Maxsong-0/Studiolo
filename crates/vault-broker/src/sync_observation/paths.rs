//! 与私有范围使用相同保守 Unicode 键；实际全部直属名字先入索引再过滤。
use crate::{BrokerError, VaultPath};
use std::collections::BTreeMap;

pub(super) fn fold(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}
pub(super) fn within(path: &VaultPath, prefix: &VaultPath) -> bool {
    let path = fold(path.as_str());
    let prefix = fold(prefix.as_str());
    path == prefix
        || path
            .strip_prefix(&prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}
pub(super) fn hard(path: &VaultPath) -> bool {
    let value = fold(path.as_str());
    let parts: Vec<_> = value.split('/').collect();
    parts.iter().any(|part| {
        part.starts_with('.')
            || matches!(
                *part,
                "raw"
                    | "ops"
                    | "workbench_data"
                    | "workbench_ephemeral"
                    | "skills"
                    | "policies"
                    | "capabilities"
                    | "sources"
            )
    }) || matches!(
        parts.last().copied(),
        Some("tasks.md" | "index.md" | "dashboard.md" | "claude.md" | "agents.md" | "skill.md")
    )
}
#[derive(Default)]
pub(super) struct Index {
    values: BTreeMap<String, String>,
    bytes: usize,
}
impl Index {
    pub(super) fn add(&mut self, path: &VaultPath) -> Result<(), BrokerError> {
        let raw = path.as_str();
        if raw.len() > 1024 || raw.split('/').count() > 64 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        if raw.chars().any(invisible)
            || raw
                .split('/')
                .any(|part| part.ends_with([' ', '.']) || part.contains(':'))
        {
            return Err(crate::sync_observation::invalid());
        }
        let mut prefix = String::new();
        for part in raw.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let key = fold(&prefix);
            if let Some(previous) = self.values.get(&key) {
                if previous != &prefix {
                    return Err(BrokerError::PathCollision {
                        requested: path.clone(),
                        existing: previous.clone(),
                    });
                }
            } else {
                self.bytes = self
                    .bytes
                    .checked_add(key.len() + prefix.len())
                    .ok_or(BrokerError::WriterLimitExceeded)?;
                if self.values.len() >= 66_048 || self.bytes > 8 * 1024 * 1024 {
                    return Err(BrokerError::WriterLimitExceeded);
                }
                self.values.insert(key, prefix.clone());
            }
        }
        Ok(())
    }
}
fn invisible(value: char) -> bool {
    value.is_control()
        || matches!(value,
        '\u{00ad}' | '\u{0600}'..='\u{0605}' | '\u{061c}' | '\u{06dd}' | '\u{070f}' |
        '\u{0890}'..='\u{0891}' | '\u{08e2}' | '\u{115f}'..='\u{1160}' |
        '\u{180b}'..='\u{180f}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' |
        '\u{2060}'..='\u{206f}' | '\u{3164}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}' |
        '\u{ffa0}' | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' |
        '\u{13430}'..='\u{13455}' | '\u{1bca0}'..='\u{1bca3}' |
        '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}' | '\u{e0100}'..='\u{e01ef}')
}
