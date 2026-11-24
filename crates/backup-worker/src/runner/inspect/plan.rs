//! 所有参数分别传给 exec；文件名可含中文/空格，不经过 shell 或 URL 再解释。
use crate::{ProcessError, Result};
use std::path::Path;
use studiolo_backup::worker::restic::{RepositoryRef, SnapshotRef};
use studiolo_backup::worker::{restic::JobTag, state::JobKey};

pub(crate) fn snapshots(repository: &RepositoryRef, key: &JobKey) -> Result<Vec<String>> {
    let tag = JobTag::for_job(key).map_err(|_| ProcessError::Configuration)?;
    Ok(vec![
        "--json".into(),
        "--no-cache".into(),
        "--repo".into(),
        repository.as_str().into(),
        "snapshots".into(),
        "--tag".into(),
        tag.as_str().into(),
    ])
}
pub(crate) fn listing(repository: &RepositoryRef, snapshot: &SnapshotRef) -> Result<Vec<String>> {
    if snapshot.as_str().len() != 64 {
        return Err(ProcessError::Configuration);
    }
    Ok(vec![
        "--json".into(),
        "--no-cache".into(),
        "--repo".into(),
        repository.as_str().into(),
        "ls".into(),
        snapshot.as_str().into(),
    ])
}

/// 整份封存一次读回：对封存根做一次 `dump`，restic 把整棵树写成一条 tar 流（BUG-053）。
/// 一个文件一个 `dump` 时，每次都要从仓库重新加载索引，1 万多个文件要几个小时。
pub(crate) fn tree(
    repository: &RepositoryRef,
    snapshot: &SnapshotRef,
    root: &Path,
) -> Result<Vec<String>> {
    if snapshot.as_str().len() != 64 {
        return Err(ProcessError::Configuration);
    }
    let text = root.to_str().ok_or(ProcessError::Configuration)?;
    if !root.is_absolute()
        || text.len() > 4096
        || text.chars().any(char::is_control)
        || text.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(ProcessError::Configuration);
    }
    Ok(vec![
        "--json".into(),
        "--no-cache".into(),
        "--repo".into(),
        repository.as_str().into(),
        "dump".into(),
        "--archive".into(),
        "tar".into(),
        snapshot.as_str().into(),
        text.into(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dump_takes_the_absolute_sealed_root_and_requires_full_snapshot() {
        let repository = RepositoryRef::parse("rest:https://backup.example.test/repo/").unwrap();
        let snapshot = SnapshotRef::parse("ab".repeat(32)).unwrap();
        let argv = tree(&repository, &snapshot, Path::new("/sealed/中文 a;$")).unwrap();
        assert_eq!(argv.len(), 9);
        assert_eq!(argv[4..7], ["dump", "--archive", "tar"]);
        assert_eq!(argv[8], "/sealed/中文 a;$");
        let short = SnapshotRef::parse("abcdef12").unwrap();
        assert!(tree(&repository, &short, Path::new("/sealed")).is_err());
        for root in [
            "relative",
            "/sealed/../other",
            "/sealed/./x",
            "/sealed/a\nb",
        ] {
            assert!(
                tree(&repository, &snapshot, Path::new(root)).is_err(),
                "{root}"
            );
        }
    }
}
