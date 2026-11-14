//! 所有参数分别传给 exec；文件名可含中文/空格，不经过 shell 或 URL 再解释。
use crate::{ProcessError, Result};
use std::path::Path;
use studiolo_backup::worker::restic::{RepositoryRef, SnapshotRef};
use studiolo_core::VaultPath;

pub(crate) fn file(
    repository: &RepositoryRef,
    snapshot: &SnapshotRef,
    root: &Path,
    relative: &VaultPath,
) -> Result<Vec<String>> {
    if snapshot.as_str().len() != 64 {
        return Err(ProcessError::Configuration);
    }
    let path = root.join(relative.as_str());
    let text = path.to_str().ok_or(ProcessError::Configuration)?;
    if !path.is_absolute()
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
        snapshot.as_str().into(),
        text.into(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dump_is_one_absolute_argument_and_requires_full_snapshot() {
        let repository = RepositoryRef::parse("rest:https://backup.example.test/repo/").unwrap();
        let snapshot = SnapshotRef::parse("ab".repeat(32)).unwrap();
        let relative = VaultPath::parse("data/中文笔记/a b;$.md").unwrap();
        let argv = file(&repository, &snapshot, Path::new("/sealed"), &relative).unwrap();
        assert_eq!(argv.len(), 7);
        assert_eq!(argv[4], "dump");
        assert_eq!(argv[6], "/sealed/data/中文笔记/a b;$.md");
        assert!(
            file(
                &repository,
                &SnapshotRef::parse("abcdef12").unwrap(),
                Path::new("/sealed"),
                &relative
            )
            .is_err()
        );
        assert!(file(&repository, &snapshot, Path::new("relative"), &relative).is_err());
    }
}
