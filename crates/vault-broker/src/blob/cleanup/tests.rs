//! 真实临时文件、重开与确定性漂移契约；大 I/O 与附件测试共用一把测试锁。
mod boundaries;
mod budgets;
mod inventory;
mod lifecycle;
use super::*;
use crate::blob::LARGE_IO;
use crate::*;
use std::{fs, io::Write, path::PathBuf, sync::atomic::Ordering};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t046-blob-cleanup-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }
    fn temporary(&self, domain: &str, bytes: &[u8]) -> PathBuf {
        let path = self
            .data
            .join(domain)
            .join(format!(".studiolo-blob-{}.tmp", VaultUuid::new()));
        let mut output = fs::File::create(&path).unwrap();
        output.write_all(bytes).unwrap();
        output.sync_all().unwrap();
        path
    }
    fn sparse(&self, domain: &str, bytes: u64) -> PathBuf {
        // 只有逻辑配额契约使用稀疏文件；完整非稀疏正文另有独立契约。
        let path = self.temporary(domain, b"");
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(bytes)
            .unwrap();
        path
    }
}
fn path() -> VaultPath {
    VaultPath::parse("note.md").unwrap()
}
fn actor() -> MutationActor {
    MutationActor::Human {
        issuer: "https://issuer.test".into(),
        subject: "human".into(),
    }
}
fn fault(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
fn count(fixture: &Fixture, domain: &str) -> usize {
    fs::read_dir(fixture.data.join(domain)).unwrap().count()
}
