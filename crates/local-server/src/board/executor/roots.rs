//! Path admission runs before recovery signals and before any Runner listener is created.
use super::RunnerConfig;
use std::path::{Path, PathBuf};

/// Validates trusted Runner mappings against every canonical, credential and control root.
/// The caller supplies its already-verified host roots; none come from an AI request.
pub fn validate_isolated_roots(config: &RunnerConfig, forbidden: &[PathBuf]) -> Result<(), String> {
    config.validate()?;
    if forbidden.is_empty() {
        return Err("缺少主机隔离边界".into());
    }
    let mut mounts = vec![config.state_root.as_path(), config.raw_root.as_path()];
    mounts.extend(
        config
            .providers
            .iter()
            .flat_map(|p| [p.attempt_root.as_path(), p.retained_root.as_path()]),
    );
    mounts.extend(config.projects.iter().map(|p| p.repository.as_path()));
    for path in &mounts {
        validate_path(path, forbidden)?;
    }
    for (index, path) in mounts.iter().enumerate() {
        if mounts[index + 1..]
            .iter()
            .any(|other| overlaps(path, other))
        {
            return Err("Runner映射根必须相互分离".into());
        }
    }
    // Read-only runtimes may be shared by providers, but never contain mutable or secret roots.
    for provider in &config.providers {
        validate_path(&provider.runtime_root, forbidden)?;
        if mounts
            .iter()
            .any(|path| overlaps(path, &provider.runtime_root))
        {
            return Err("只读runtime与Runner可写根重叠".into());
        }
    }
    Ok(())
}
fn validate_path(path: &Path, forbidden: &[PathBuf]) -> Result<(), String> {
    if !path.is_absolute() || std::fs::canonicalize(path).map_err(|_| "Runner映射根不可用")? != path
    {
        return Err("Runner映射根必须为规范绝对路径".into());
    }
    if forbidden
        .iter()
        .any(|root| !root.is_absolute() || overlaps(path, root))
    {
        return Err("Runner根与受保护主机路径重叠".into());
    }
    Ok(())
}
fn overlaps(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}
