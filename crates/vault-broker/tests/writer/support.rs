//! 只操作测试自有目录；目录清单用于证明拒绝未创建、修复或安装内容。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::FsVaultBroker;
use tempfile::TempDir;

pub(super) struct Fixture {
    pub directory: TempDir,
    pub vault: PathBuf,
    pub data: PathBuf,
}

impl Fixture {
    pub fn empty() -> Self {
        let directory = TempDir::new().unwrap();
        // macOS 的临时目录可能经 /var 别名进入，夹具统一使用真实路径。
        let base = directory.path().canonicalize().unwrap();
        Self {
            vault: base.join("vault"),
            data: base.join("data"),
            directory,
        }
    }

    pub fn initialized() -> Self {
        let fixture = Self::empty();
        drop(FsVaultBroker::open(&fixture.vault, &fixture.data).unwrap());
        fixture
    }

    pub fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.vault, &self.data).unwrap()
    }

    pub fn marker(&self) -> PathBuf {
        self.vault.join(".workbench/vault.json")
    }
}

pub(super) fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Entry {
    File(Vec<u8>),
    Directory,
    Link(PathBuf),
    Other,
}

pub(super) fn inventory(root: &Path) -> BTreeMap<PathBuf, Entry> {
    fn visit(root: &Path, current: &Path, entries: &mut BTreeMap<PathBuf, Entry>) {
        for entry in fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if metadata.file_type().is_symlink() {
                entries.insert(relative, Entry::Link(fs::read_link(path).unwrap()));
            } else if metadata.is_dir() {
                entries.insert(relative, Entry::Directory);
                visit(root, &path, entries);
            } else if metadata.is_file() {
                entries.insert(relative, Entry::File(fs::read(path).unwrap()));
            } else {
                entries.insert(relative, Entry::Other);
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}
