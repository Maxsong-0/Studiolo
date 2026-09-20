//! 通用 batch 也执行记忆准入；不能绕开专用预览创建另一份同义记忆。
use super::{plan, scan};
use crate::{BatchDiff, BrokerError, FsVaultBroker, MemoryVaultBroker, VaultObject, VaultPath};
use std::collections::{BTreeMap, BTreeSet};
fn domain(path: &VaultPath) -> Option<String> {
    let mut parts = Vec::new();
    let mut iter = path.as_str().split('/').peekable();
    while let Some(part) = iter.next() {
        parts.push(part);
        if part == "03-记忆" && iter.peek().is_some() {
            return Some(parts.join("/").to_lowercase());
        }
    }
    None
}
pub(crate) fn filesystem(broker: &FsVaultBroker, diffs: &[BatchDiff]) -> Result<(), BrokerError> {
    if !diffs.iter().any(|diff| domain(&diff.path).is_some()) {
        return Ok(());
    }
    check(diffs, scan::filesystem(broker)?)
}
pub(crate) fn memory(broker: &MemoryVaultBroker, diffs: &[BatchDiff]) -> Result<(), BrokerError> {
    if !diffs.iter().any(|diff| domain(&diff.path).is_some()) {
        return Ok(());
    }
    check(diffs, scan::memory(broker)?)
}
fn check(diffs: &[BatchDiff], records: Vec<VaultObject>) -> Result<(), BrokerError> {
    let replaced: BTreeSet<_> = diffs.iter().map(|diff| diff.path.collision_key()).collect();
    let mut candidates = BTreeMap::new();
    for diff in diffs {
        let Some(domain) = domain(&diff.path) else {
            continue;
        };
        if diff.after.lines().count() > 300 {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let metadata = plan::frontmatter(diff.after.as_bytes())?
            .ok_or_else(|| plan::invalid("记忆必须含结构化 frontmatter"))?;
        let date = metadata
            .get("date")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| plan::invalid("记忆必须含绝对日期"))?;
        plan::date(date)?;
        let key = plan::existing_key(diff.after.as_bytes())?
            .ok_or_else(|| plan::invalid("记忆必须含类型与摘要"))?;
        if candidates.insert((domain, key), &diff.path).is_some() {
            return Err(plan::invalid("同批含重复记忆"));
        }
    }
    for object in records {
        if replaced.contains(&object.path.collision_key()) {
            continue;
        }
        let Some(domain) = domain(&object.path) else {
            continue;
        };
        if !candidates
            .keys()
            .any(|(candidate_domain, _)| candidate_domain == &domain)
        {
            continue;
        }
        if let Some(key) = plan::existing_key(&object.content)?
            && candidates.contains_key(&(domain, key))
        {
            return Err(plan::invalid("记忆类型与摘要已存在，必须修改同一条"));
        }
    }
    Ok(())
}
