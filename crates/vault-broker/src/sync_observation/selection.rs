//! 读取选择没有授权语义；先完整验证数据，再接触实际根和元信息。
use super::{SyncReadSelection, paths};
use crate::{BrokerError, VaultPath};
use std::collections::BTreeSet;

impl SyncReadSelection {
    pub(super) fn validate(&self, required: &[VaultPath]) -> Result<paths::Index, BrokerError> {
        if self.folders.len() > 64
            || self.files.len() > 128
            || self.excluded_prefixes.len() > 320
            || required.len() > 4096
            || (self.folders.is_empty() && self.files.is_empty())
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let mut index = paths::Index::default();
        let mut rules = Vec::new();
        for rule in self.folders.iter().chain(&self.files) {
            index.add(&rule.path)?;
            if rule.max_bytes == 0 || rule.max_bytes > 256 * 1024 * 1024 {
                return Err(BrokerError::WriterLimitExceeded);
            }
            if self.excluded(&rule.path)
                || rules.iter().any(|other| {
                    paths::within(&rule.path, other) || paths::within(other, &rule.path)
                })
            {
                return Err(crate::sync_observation::invalid());
            }
            rules.push(rule.path.clone());
        }
        let mut excluded = BTreeSet::new();
        for path in &self.excluded_prefixes {
            index.add(path)?;
            if !excluded.insert(path)
                || self
                    .files
                    .iter()
                    .any(|rule| paths::within(path, &rule.path))
            {
                return Err(crate::sync_observation::invalid());
            }
        }
        let mut seen = BTreeSet::new();
        for path in required {
            index.add(path)?;
            if !seen.insert(path) || self.file_limit(path).is_none() {
                return Err(crate::sync_observation::invalid());
            }
        }
        Ok(index)
    }
    pub(super) fn excluded(&self, path: &VaultPath) -> bool {
        paths::hard(path)
            || self
                .excluded_prefixes
                .iter()
                .any(|prefix| paths::within(path, prefix))
    }
    pub(super) fn file_limit(&self, path: &VaultPath) -> Option<u64> {
        if self.excluded(path) {
            return None;
        }
        self.files
            .iter()
            .find(|rule| rule.path == *path)
            .map(|rule| rule.max_bytes)
            .or_else(|| {
                self.folders
                    .iter()
                    .find(|rule| rule.path != *path && paths::within(path, &rule.path))
                    .map(|rule| rule.max_bytes)
            })
    }
    pub(super) fn traverse(&self, path: &VaultPath) -> bool {
        !self.excluded(path)
            && (self
                .folders
                .iter()
                .any(|rule| paths::within(path, &rule.path) || paths::within(&rule.path, path))
                || self
                    .files
                    .iter()
                    .any(|rule| rule.path != *path && paths::within(&rule.path, path)))
    }
    pub(super) fn needed_directory(&self, path: &VaultPath) -> bool {
        self.folders
            .iter()
            .any(|rule| paths::within(&rule.path, path))
            || self
                .files
                .iter()
                .any(|rule| rule.path != *path && paths::within(&rule.path, path))
    }
}
