//! 真实文件、故障阶段和子进程内存契约；只有主线执行后才构成验收证据。
mod bindings;
mod failures;
mod history;
mod history_pin_restore;
mod history_pins;
mod large;
mod legacy_history_pins;
mod lifecycle;
mod limits;
mod memory;
mod recovery;
use super::*;
use crate::blob::LARGE_IO as LARGE;
use crate::*;
use std::{fs, io::Write, path::PathBuf, sync::atomic::Ordering};
const LARGE_BYTES: usize = 200 * 1024 * 1024 + 17;
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t046-stream-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            source,
            _temp: temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn reopen(&self) -> FsVaultBroker {
        FsVaultBroker::open_existing(&self.root, &self.data).unwrap()
    }
    fn bytes(&self, value: &[u8]) -> BlobSource {
        fs::write(self.source.join("input.bin"), value).unwrap();
        self.input()
    }
    fn pattern(&self, bytes: usize) -> BlobSource {
        let mut output = fs::File::create(self.source.join("input.bin")).unwrap();
        let chunk: Vec<u8> = (0..65536).map(|i| ((i * 17 + 3) % 251) as u8).collect();
        for _ in 0..bytes / chunk.len() {
            output.write_all(&chunk).unwrap();
        }
        output.write_all(&chunk[..bytes % chunk.len()]).unwrap();
        output.sync_all().unwrap();
        self.input()
    }
    fn input(&self) -> BlobSource {
        BlobSource::open(&self.source, VaultPath::parse("input.bin").unwrap()).unwrap()
    }
}
fn path() -> VaultPath {
    VaultPath::parse("attachments/payload.bin").unwrap()
}
fn limits() -> StreamLimits {
    StreamLimits::new(crate::blob::OBJECT_LIMIT).unwrap()
}
fn actor() -> MutationActor {
    MutationActor::Human {
        issuer: "https://issuer.test".into(),
        subject: "human".into(),
    }
}
fn metadata() -> MutationMetadata {
    MutationMetadata::new(actor(), MutationKind::Replace).unwrap()
}
fn request(source: &BlobSource, key: &str, precondition: Precondition) -> StreamMutation {
    StreamMutation {
        path: path(),
        precondition,
        idempotency_key: key.into(),
        content: source.identity(limits()).unwrap(),
    }
}
fn ack(broker: &mut FsVaultBroker) {
    for _ in 0..2 {
        for candidate in broker.pending_content_audit(64).unwrap() {
            let offset = if candidate.event().phase == ContentAuditPhase::Intent {
                1
            } else {
                2
            };
            broker
                .confirm_content_audit(candidate, offset, &"a".repeat(64))
                .unwrap();
        }
    }
}
fn fault(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
fn stage_files(fixture: &Fixture) -> Vec<PathBuf> {
    fs::read_dir(fixture.root.join(".workbench/staging"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}
fn installed(broker: &FsVaultBroker) -> crate::blob::file::ContentProof {
    crate::blob::file::scan(
        &broker.root,
        &path(),
        crate::blob::OBJECT_LIMIT,
        Some((broker.vault_uuid, &path())),
    )
    .unwrap()
}
