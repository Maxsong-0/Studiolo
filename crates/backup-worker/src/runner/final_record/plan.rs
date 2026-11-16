//! 最终记录是固定虚拟路径；stdin 不接触宿主机文件，也不允许命令替代输入。
use crate::{ProcessError, Result};
use std::path::PathBuf;
use studiolo_backup::worker::{
    restic::{JobTag, RepositoryRef, SnapshotRef},
    state::JobKey,
};

pub(crate) struct Marker {
    pub tag: JobTag,
    pub path: PathBuf,
}
impl Marker {
    pub fn for_job(key: &JobKey) -> Result<Self> {
        let original = JobTag::for_job(key).map_err(|_| ProcessError::Configuration)?;
        let tag = JobTag::parse(format!("{}.final", original.as_str()))
            .map_err(|_| ProcessError::Configuration)?;
        let path = PathBuf::from(format!("/studiolo-final-{}.json", original.as_str()));
        Ok(Self { tag, path })
    }
    fn globals(&self, repository: &RepositoryRef) -> Vec<String> {
        vec![
            "--json".into(),
            "--no-cache".into(),
            "--repo".into(),
            repository.as_str().into(),
        ]
    }
    pub fn snapshots(&self, repository: &RepositoryRef) -> Vec<String> {
        let mut argv = self.globals(repository);
        argv.extend(["snapshots".into(), "--tag".into(), self.tag.as_str().into()]);
        argv
    }
    pub fn backup(&self, repository: &RepositoryRef) -> Vec<String> {
        let mut argv = self.globals(repository);
        argv.extend([
            "backup".into(),
            "--stdin".into(),
            "--stdin-filename".into(),
            self.path.to_string_lossy().into_owned(),
            "--tag".into(),
            self.tag.as_str().into(),
        ]);
        argv
    }
    pub fn dump(&self, repository: &RepositoryRef, snapshot: &SnapshotRef) -> Result<Vec<String>> {
        if snapshot.as_str().len() != 64 {
            return Err(ProcessError::Protocol);
        }
        let mut argv = self.globals(repository);
        argv.extend([
            "dump".into(),
            snapshot.as_str().into(),
            self.path.to_string_lossy().into_owned(),
        ]);
        Ok(argv)
    }
}
