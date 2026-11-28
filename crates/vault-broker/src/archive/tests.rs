//! 真实临时目录上的打包契约；账本回执是模拟的，封存水位由用例给。
mod batches;
mod bounded;
mod burst;
mod crashes;
mod eligibility;
mod file_terminals;
mod lifecycle;
mod renames;
mod scale;
mod seal;
mod unaudited;
mod unpacked;

use super::{Part, format, loose};
use crate::{
    content_audit::store as events,
    mutation::record::{Outcome, Terminal},
    recorded::envelope::{Envelope, encode},
    records::WriterRecords,
    *,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    sync::atomic::Ordering,
};

const DAY: i64 = 24 * 60 * 60 * 1000;
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Fixture {
    temp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::Builder::new()
            .prefix("studiolo-t235-archive-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir()).unwrap())
            .unwrap();
        Self {
            root: temp.path().join("vault"),
            data: temp.path().join("data"),
            temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.root, &self.data).unwrap()
    }
    fn try_reopen(&self) -> Result<FsVaultBroker, BrokerError> {
        FsVaultBroker::open_existing(&self.root, &self.data)
    }
    fn reopen(&self) -> FsVaultBroker {
        self.try_reopen().unwrap()
    }
    fn target(&self, name: &str) -> PathBuf {
        let path = self.temp.path().join(name);
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn state(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.data.join("writer-state.json")).unwrap()).unwrap()
    }
    /// 段文件，按名字排。
    fn segments(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(self.data.join("archive-v1")) else {
            return Vec::new();
        };
        let mut paths: Vec<_> = entries.map(|entry| entry.unwrap().path()).collect();
        paths.sort();
        paths
    }
}

fn logical(name: &str) -> VaultPath {
    VaultPath::parse(name).unwrap()
}
fn metadata() -> MutationMetadata {
    MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap()
}
fn now() -> i64 {
    crate::history::store::now().unwrap()
}
/// 八天之后再看，封存盖到了最后：真写出来的记录全部合格。
fn later() -> ArchiveCutoff {
    ArchiveCutoff {
        sealed_offset: 1_000_000,
        now_ms: now() + 8 * DAY,
    }
}
fn note(index: usize) -> Mutation {
    Mutation::create(
        logical(&format!("note-{index}.md")),
        format!("正文 {index}").into_bytes(),
        format!("key-{index}"),
    )
}
/// 真写 `count` 篇、回执全落。
fn notes(broker: &mut FsVaultBroker, count: usize) {
    for index in 0..count {
        broker.mutate_recorded(note(index), metadata()).unwrap();
    }
    flush(broker);
}
fn flush(broker: &mut FsVaultBroker) {
    let mut offset = 1;
    loop {
        let batch = broker.pending_content_audit(64).unwrap();
        if batch.is_empty() {
            return;
        }
        for candidate in batch {
            broker
                .confirm_content_audit(candidate, offset, HASH)
                .unwrap();
            offset += 1;
        }
    }
}
fn fault(broker: &FsVaultBroker, point: u8) {
    broker.writer.fault.store(point, Ordering::Release);
}
/// 打一轮，交回一共打了几包；出错就停在那一步。
fn round(
    broker: &mut FsVaultBroker,
    cutoff: ArchiveCutoff,
    batch: ArchiveBatch,
) -> Result<usize, BrokerError> {
    let mut cursor = ArchiveCursor::default();
    let mut packed = 0;
    loop {
        let step = broker.archive_step(cutoff, batch, &mut cursor)?;
        packed += step.packed;
        if step.done {
            return Ok(packed);
        }
    }
}
/// 像离线命令那样打完一轮：剩多少写多少。
fn pack(broker: &mut FsVaultBroker, cutoff: ArchiveCutoff) -> usize {
    round(broker, cutoff, ArchiveBatch::All).unwrap()
}
/// 打一轮，中途必定断开；交回断在哪个错上。
fn interrupted(broker: &mut FsVaultBroker) -> BrokerError {
    round(broker, later(), ArchiveBatch::All).unwrap_err()
}
/// `archive-v1/` 里原子写断开留下的临时文件。
fn residues(fixture: &Fixture) -> Vec<PathBuf> {
    fixture
        .segments()
        .into_iter()
        .filter(|path| path.file_name().unwrap().to_str().unwrap().starts_with('.'))
        .collect()
}
type Records = BTreeMap<(String, String), Vec<u8>>;
/// 现在的散件。
fn loose_records(data: &Path) -> Records {
    let mut result = Records::new();
    for path in loose::DIRECTORIES {
        let Ok(entries) = fs::read_dir(data.join(path)) else {
            continue;
        };
        for entry in entries {
            let entry = entry.unwrap();
            let name = entry.file_name().into_string().unwrap();
            let bytes = fs::read(entry.path()).unwrap();
            assert!(result.insert((path.to_owned(), name), bytes).is_none());
        }
    }
    result
}
/// 散件和段合起来的全部记录；两处都有的必须一样。打包、断电、还原前后这一份都不许变。
fn all_records(broker: &FsVaultBroker) -> Records {
    let mut result = loose_records(&broker_data(broker));
    let archive = broker.archive().unwrap();
    for pack in archive.packs() {
        for part in pack.kind.parts() {
            let name = pack.name_of(part);
            let bytes = archive.read(&broker.data, part, name).unwrap().unwrap();
            let key = (part.directory().to_owned(), part.file_name(name));
            if let Some(kept) = result.insert(key, bytes.clone()) {
                assert_eq!(kept, bytes);
            }
        }
    }
    result
}
fn broker_data(broker: &FsVaultBroker) -> PathBuf {
    broker.writer.checkpoint_paths().1.to_owned()
}
/// 旧键答得出原回执，同一个键换了内容报冲突。
fn answers(broker: &FsVaultBroker, index: usize) {
    let request = note(index);
    let receipt = broker
        .mutation_replay_recorded(&request, &metadata())
        .unwrap()
        .unwrap();
    assert_eq!(receipt.path, request.path);
    let mut changed = request;
    changed.content.push(b'!');
    assert!(matches!(
        broker.mutation_replay_recorded(&changed, &metadata()),
        Err(BrokerError::IdempotencyConflict)
    ));
}

/// 合成一条已提交、两份回执都在、写于八天前的正文记录：按真编码直接落盘，不走几千次真写。
fn bulk_request(index: usize) -> Mutation {
    Mutation::create(
        logical(&format!("bulk/{index}.md")),
        format!("合成 {index}").into_bytes(),
        format!("bulk-{index}"),
    )
}
/// 合成的没有审计主体的写入（看板投影那种）：换一个目录和键，免得和有主体的撞。
fn unaudited_request(index: usize) -> Mutation {
    Mutation::create(
        logical(&format!("board/bulk-{index}.md")),
        format!("任务 {index}").into_bytes(),
        format!("board-bulk-{index}"),
    )
}
/// 合成记录的样子：写于何时、有没有审计主体。
#[derive(Clone, Copy)]
struct Shape {
    at: i64,
    audited: bool,
}
impl Shape {
    /// 八天前、有审计主体：和 [`bulk`] 一样。
    fn old() -> Self {
        Self {
            at: now() - 8 * DAY,
            audited: true,
        }
    }
    /// 八天前、没有审计主体。
    fn unaudited() -> Self {
        Self {
            audited: false,
            ..Self::old()
        }
    }
    /// 一天前、有审计主体：不满 7 天。
    fn young() -> Self {
        Self {
            at: now() - DAY,
            ..Self::old()
        }
    }
}
fn bulk(broker: &FsVaultBroker, range: std::ops::Range<usize>) {
    bulk_with(broker, range, Shape::old());
}
fn bulk_with(broker: &FsVaultBroker, range: std::ops::Range<usize>, shape: Shape) {
    let data = broker_data(broker);
    let requests = data.join("history/meta-v1/requests");
    let at = shape.at;
    for index in range {
        let request = if shape.audited {
            bulk_request(index)
        } else {
            unaudited_request(index)
        };
        let key_hash = sha256(request.idempotency_key.as_bytes());
        let envelope = Envelope {
            version: 1,
            pair_id: broker.writer.binding.pair_id,
            vault_uuid: broker.vault_uuid,
            id: crate::recorded::store::id(broker, &key_hash),
            fingerprint: FsVaultBroker::fingerprint(&request).unwrap(),
            path: request.path.clone(),
            precondition: request.precondition,
            target: Revision::compute(broker.vault_uuid, &request.path, &request.content),
            sha256: sha256(&request.content),
            bytes: request.content.len() as u64,
            requested_at_ms: Some(at),
            actor: shape.audited.then_some(MutationActor::Obsidian),
            kind: shape.audited.then_some(MutationKind::Replace),
            key_hash,
        };
        let terminal = Terminal {
            recording: Some(envelope.reference().unwrap()),
            format_version: 4,
            pair_id: envelope.pair_id,
            sequence: Some(1),
            fingerprint: envelope.fingerprint.clone(),
            expected_revision: envelope.target,
            outcome: Outcome::Committed {
                receipt: MutationReceipt {
                    path: envelope.path.clone(),
                    revision: envelope.target,
                    previous_revision: None,
                },
            },
        };
        let id = &envelope.id;
        fs::write(
            requests.join(format!("{id}.json")),
            encode(&envelope).unwrap(),
        )
        .unwrap();
        // 没有审计主体的写入没有回执。
        let receipts = if shape.audited {
            vec![
                (None, 2 * index as u64 + 1),
                (Some(ContentAuditOutcome::Committed), 2 * index as u64 + 2),
            ]
        } else {
            Vec::new()
        };
        for (outcome, offset) in receipts {
            let row = events::row(&envelope, outcome).unwrap();
            let ack = events::Ack {
                version: 1,
                pair_id: envelope.pair_id,
                event_id: row.event.event_id.clone(),
                phase: row.event.phase,
                digest: events::digest(&row).unwrap(),
                offset,
                record_hash: HASH.to_owned(),
            };
            fs::write(
                requests.join(format!("{}.ack", events::name(&envelope, row.event.phase))),
                serde_json::to_vec(&ack).unwrap(),
            )
            .unwrap();
        }
        fs::write(
            data.join(format!("idempotency/{}.json", envelope.key_hash)),
            serde_json::to_vec(&terminal).unwrap(),
        )
        .unwrap();
    }
}
/// 没有描述符的旧格式重放（169 现在有 138 条），永远留散件。
fn legacy_replays(broker: &FsVaultBroker, range: std::ops::Range<usize>) {
    let data = broker_data(broker);
    let revision = Revision::compute(broker.vault_uuid, &logical("old.md"), b"old");
    for index in range {
        fs::write(
            data.join(format!(
                "idempotency/{}.json",
                sha256(format!("legacy-{index}").as_bytes())
            )),
            serde_json::to_vec(&serde_json::json!({
                "fingerprint": "c".repeat(64),
                "expected_revision": revision,
                "receipt": {
                    "path": "old.md",
                    "revision": revision,
                    "previous_revision": null,
                },
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
fn deadline() -> std::time::Instant {
    std::time::Instant::now() + std::time::Duration::from_secs(120)
}
/// 和协调器同一条路封存一次，交回核过的回执数；写者原样返还。
fn checkpoint(
    fixture: &Fixture,
    broker: FsVaultBroker,
    name: &str,
) -> (FsVaultBroker, Result<u64, BrokerError>) {
    let mut frozen = broker.freeze_checkpoint(deadline());
    let result = (|| {
        frozen.capture()?;
        let exported = frozen.take_capture()?.export(
            &fixture.target(&format!("{name}-vault")),
            &fixture.target(&format!("{name}-data")),
            deadline(),
        )?;
        let (sealed, coverage) = exported.verify(deadline(), |_, offset, hash| {
            if offset == 0 || hash != HASH {
                return Err(BrokerError::CorruptWriterState);
            }
            Ok(())
        })?;
        assert!(sealed.is_verified());
        Ok(coverage.checked())
    })();
    (frozen.thaw(), result)
}
