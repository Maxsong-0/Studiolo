//! 只复验本作业原清单中的文件，不提供任意路径读取或恢复落盘能力。
pub(crate) mod listing;
pub(crate) mod plan;
pub(crate) mod snapshot;
use super::*;
use std::path::Path;
use studiolo_backup::{
    checkpoint::staging::SealedInventory,
    worker::restic::{RepositoryRef, SnapshotRef},
};

impl AppendRunner {
    /// 流式比对完整封存的全部字节；仍需另核快照作业标签、目录漏项/多项及远端最终清单。
    pub fn verify_snapshot_files(&self, snapshot: &SnapshotRef) -> Result<()> {
        self.source.check()?;
        verify_files(
            self.source.repository(),
            snapshot,
            &self.sealed().ticket().transfer_source.root,
            self.sealed().inventory(),
            &|argv, expected| {
                self.runtime
                    .verify_tree(argv, expected, self.source.deadline(), &|| {
                        self.source.check_time()
                    })
            },
        )?;
        self.source.check()
    }
}

/// 读回整份封存：只起一个 restic，对封存根做一次 `dump`，整棵树的 tar 流边读边按清单核
/// （BUG-053：一个文件一个 `dump` 时，每次都要从仓库重新加载索引，1 万多个文件核不完）。
/// `run` 起进程并核输出，测试里换成替身。
pub(crate) fn verify_files(
    repository: &RepositoryRef,
    snapshot: &SnapshotRef,
    root: &Path,
    inventory: &SealedInventory,
    run: &dyn Fn(&[String], &process::TreeExpectation) -> Result<()>,
) -> Result<()> {
    let argv = plan::tree(repository, snapshot, root)?;
    let mut expected = process::TreeExpectation::new(root)?;
    for directory in inventory.directories() {
        expected.directory(directory)?;
    }
    for file in inventory.files() {
        expected.file(file.path().as_str(), file.bytes(), file.sha256())?;
    }
    run(&argv, &expected)
}

#[cfg(test)]
#[path = "../../tests/support/inspect_files.rs"]
mod tests;
