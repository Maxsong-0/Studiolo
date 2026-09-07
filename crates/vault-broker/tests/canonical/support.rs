//! Shared synthetic roots initialized only by the existing trusted Fs constructor.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{CanonicalReadVault, FsVaultBroker};
use tempfile::TempDir;

pub(super) struct Fixture {
    pub directory: TempDir,
    pub root: PathBuf,
    pub broker: FsVaultBroker,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let root = directory.path().join("vault");
        let broker = FsVaultBroker::open(&root, &directory.path().join("data")).unwrap();
        Self {
            directory,
            root,
            broker,
        }
    }

    pub fn reader(&self) -> CanonicalReadVault {
        CanonicalReadVault::open_existing(&self.root).unwrap()
    }

    pub fn marker(&self) -> PathBuf {
        self.root.join(".workbench/vault.json")
    }
}

pub(super) fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

pub(super) fn create_fifo(path: &Path) {
    // rustix exposes mkfifoat on Linux, but not on the macOS target.
    assert!(
        std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
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
