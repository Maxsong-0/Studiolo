//! 路径准入先于恢复信号执行，也先于任何 `Runner` 监听的创建。
use super::RunnerConfig;
use std::path::{Path, PathBuf};

/// 用每个 `canonical` 根、凭据根与控制根校验受信任的 `Runner` 映射。
/// 宿主机根由调用方提供且已核验；没有一个来自 AI 请求。
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
    // 只读运行时可由多个提供方共用，但绝不能包含可变根或秘密根。
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
