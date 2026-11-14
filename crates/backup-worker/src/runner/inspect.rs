//! 只复验本作业原清单中的文件，不提供任意路径读取或恢复落盘能力。
pub(crate) mod listing;
pub(crate) mod plan;
pub(crate) mod snapshot;
use super::*;
use studiolo_backup::worker::restic::SnapshotRef;

impl AppendRunner {
    /// 逐文件流式比对完整封存；仍需另核快照作业标签、目录漏项/多项及远端最终清单。
    pub fn verify_snapshot_files(&self, snapshot: &SnapshotRef) -> Result<()> {
        if self.poisoned.get() {
            return Err(ProcessError::CleanupFailed);
        }
        self.source.check()?;
        let root = &self.sealed().ticket().transfer_source.root;
        for file in self.sealed().inventory().files() {
            let argv = plan::file(self.source.repository(), snapshot, root, file.path())?;
            self.temporary.check()?;
            let result = process::verify_digest(
                &self.package,
                &argv,
                &self.credentials,
                self.temporary.path(),
                self.source.deadline(),
                &|| self.source.check_time(),
                process::DigestExpectation {
                    bytes: file.bytes(),
                    sha256: file.sha256(),
                },
            );
            if matches!(result, Err(ProcessError::CleanupFailed)) {
                self.poisoned.set(true);
            }
            result?;
            self.temporary.check()?;
        }
        self.source.check()
    }
}
