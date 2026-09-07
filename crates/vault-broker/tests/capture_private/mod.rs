//! 私有故障点只用于虚构临时目录；公开行为仍通过真实 Broker 执行。

mod binding;
mod pending;
mod reads;
mod watermark;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

use crate::{
    BrokerError, CanonicalCapture, FsVaultBroker, Mutation, ScanLimits, VaultPath, VaultReader,
    VaultWriter,
};

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            temp,
        }
    }

    fn initialize(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }

    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }

    fn journal(&self) -> Vec<u8> {
        fs::read(self.data.join("writer-state.json")).unwrap()
    }

    fn restore_journal(&self, bytes: &[u8]) {
        fs::write(self.data.join("writer-state.json"), bytes).unwrap();
    }
}

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

fn request() -> Mutation {
    Mutation::create(path("note.md"), b"intended".to_vec(), "capture-key")
}

fn capture(broker: &mut FsVaultBroker) -> CanonicalCapture {
    broker.capture(ScanLimits::default()).unwrap()
}

fn interrupt(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}

fn identity(path: &Path) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = fs::metadata(path).unwrap();
    (metadata.dev(), metadata.ino())
}

fn inventory(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, entries: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            let content = if kind.is_file() {
                fs::read(&path).unwrap()
            } else {
                Vec::new()
            };
            entries.push((path.strip_prefix(root).unwrap().to_owned(), content));
            if kind.is_dir() {
                visit(root, &path, entries);
            }
        }
    }
    let mut entries = Vec::new();
    visit(root, root, &mut entries);
    entries.sort();
    entries
}

// 仅在当前测试线程、真实读取结束后运行一次，确定性核验生产尾部检查。
thread_local! {
    static READ_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
        std::cell::RefCell::new(None)
    };
}

struct ReadHook;

fn read_hook(hook: impl FnOnce() + 'static) -> ReadHook {
    READ_HOOK.with(|slot| assert!(slot.borrow_mut().replace(Box::new(hook)).is_none()));
    ReadHook
}

pub(super) fn after_read() {
    let hook = READ_HOOK.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

impl Drop for ReadHook {
    fn drop(&mut self) {
        READ_HOOK.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}
