//! 双向固定排除与全部路径前缀检查，不依据正文哨兵决定文件权限。

use super::SyncScopeError;
use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;

pub(super) fn key(path: &VaultPath) -> String {
    fold(path.as_str())
}

fn fold(value: &str) -> String {
    // 大写再小写还会保守合并 ß/SS、ς/σ 和 Unicode 连字；不声称这是所有 FS 的规则。
    value
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn within(path: &VaultPath, prefix: &VaultPath) -> bool {
    let path = key(path);
    let prefix = key(prefix);
    path == prefix
        || path
            .strip_prefix(&prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

pub(super) fn hard_excluded(path: &VaultPath) -> bool {
    // `_规则/` 自 2026-09-23 起进同步范围（用户拍板，T-132 P1）；人侧保存的保护在 human_content 里另算。
    if crate::human_content::reserved_for_agents(path) {
        return true;
    }
    let value = key(path);
    let parts: Vec<_> = value.split('/').collect();
    // 隐藏私有文件/目录与 raw 全拒；sources 的全部范围也沿用人写范围硬排除。
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
    prefixes: BTreeMap<String, String>,
    bytes: usize,
}

impl Index {
    pub(super) fn add(&mut self, path: &VaultPath) -> Result<(), SyncScopeError> {
        self.add_bounded(path, 1024)
    }

    pub(super) fn add_bounded(
        &mut self,
        path: &VaultPath,
        limit: usize,
    ) -> Result<(), SyncScopeError> {
        let raw = path.as_str();
        if raw.len() > limit || raw.split('/').count() > 64 {
            return Err(SyncScopeError::Budget);
        }
        if raw.chars().any(invisible)
            || raw
                .split('/')
                .any(|part| part.ends_with([' ', '.']) || part.contains(':'))
        {
            return Err(SyncScopeError::Collision);
        }
        let mut prefix = String::new();
        for part in raw.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            let key = fold(&prefix);
            if let Some(previous) = self.prefixes.get(&key) {
                if previous != &prefix {
                    return Err(SyncScopeError::Collision);
                }
            } else {
                self.bytes = self
                    .bytes
                    .checked_add(key.len() + prefix.len())
                    .ok_or(SyncScopeError::Budget)?;
                if self.prefixes.len() >= 16_384 || self.bytes > 4 * 1024 * 1024 {
                    return Err(SyncScopeError::Budget);
                }
                self.prefixes.insert(key, prefix.clone());
            }
        }
        Ok(())
    }
}

pub(super) fn files(paths: &BTreeSet<VaultPath>) -> Result<(), SyncScopeError> {
    let files: BTreeSet<&str> = paths.iter().map(VaultPath::as_str).collect();
    for path in paths {
        for (index, _) in path.as_str().match_indices('/') {
            if files.contains(&path.as_str()[..index]) {
                return Err(SyncScopeError::Collision);
            }
        }
    }
    Ok(())
}

fn invisible(value: char) -> bool {
    value.is_control()
        || matches!(value,
        '\u{00ad}' | '\u{0600}'..='\u{0605}' | '\u{061c}' | '\u{06dd}' | '\u{070f}' |
        '\u{0890}'..='\u{0891}' | '\u{08e2}' | '\u{115f}'..='\u{1160}' |
        '\u{180b}'..='\u{180f}' | '\u{200b}'..='\u{200f}' |
        '\u{2028}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{3164}' |
        '\u{fe00}'..='\u{fe0f}' | '\u{feff}' | '\u{ffa0}' | '\u{fff9}'..='\u{fffb}' |
        '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{13455}' | '\u{1bca0}'..='\u{1bca3}' |
        '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}' |
        '\u{e0100}'..='\u{e01ef}')
}
