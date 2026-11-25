//! 段的内存索引：打开时沿摘要链整个核一遍，之后读一条只取那一段字节、再核一次摘要。
//!
//! 从段内容推出来的几张表也在这里：已提交正文的路径、改名的源和目标。打包不能让依赖它们的判断变松。
use super::format::{self, DIRECTORY, Head, Kind, MAX_SEGMENTS, Part, SEGMENT_BYTES};
use crate::{
    BrokerError, FileOperationKind, FileOperationOutcome, FilePresence, FsVaultBroker, Revision,
    VaultPath, VaultUuid, file_operation,
    fs::is_exact_temporary_name,
    mutation::{
        record::Outcome,
        replay::{self, Replay},
    },
    recorded, sha256,
    writer::{directory, disk},
};
use cap_std::fs::Dir;
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Seek, SeekFrom},
};

/// 一条记录的键：种类加 32 字节的号。
type Key = (Part, [u8; 32]);

#[derive(Clone)]
struct Slot {
    segment: usize,
    offset: u32,
    bytes: u32,
    sha256: [u8; 32],
}
/// 打了包的一次改名：两头的路径和当时的 revision。墓碑名额、保留字节和历史清理的保护都按它算。
#[derive(Clone)]
pub(crate) struct Rename {
    pub source: VaultPath,
    pub target: VaultPath,
    pub source_revision: Option<Revision>,
    pub target_revision: Option<Revision>,
}
/// 一包是谁；封存逐包核的时候按这个走。
#[derive(Clone)]
pub(crate) struct PackRef {
    pub kind: Kind,
    pub id: String,
    pub key: String,
}
/// 核过的段链。没打过包就是空的。
#[derive(Clone, Default)]
pub(crate) struct Archive {
    head: Option<Head>,
    segments: Vec<(String, u64)>,
    slots: HashMap<Key, Slot>,
    packs: Vec<PackRef>,
    committed: HashSet<VaultPath>,
    renames: Vec<Rename>,
    file_bytes: u64,
}
/// 读进来的段，外加目录里不在链上的段文件名（没提交或已经还原、还没清掉的）。
pub(crate) struct Loaded {
    pub archive: Archive,
    pub orphans: Vec<String>,
}

fn key(part: Part, name: &str) -> Result<Key, BrokerError> {
    let mut value = [0u8; 32];
    if !disk::hex(name) {
        return Err(BrokerError::CorruptWriterState);
    }
    for (index, byte) in value.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&name[index * 2..index * 2 + 2], 16)
            .map_err(|_| BrokerError::CorruptWriterState)?;
    }
    Ok((part, value))
}

impl Archive {
    pub(crate) fn head(&self) -> Option<&Head> {
        self.head.as_ref()
    }
    /// 这个段文件在链上。
    pub(super) fn has_segment(&self, name: &str) -> bool {
        self.segments.iter().any(|(own, _)| own == name)
    }
    pub(crate) fn packs(&self) -> &[PackRef] {
        &self.packs
    }
    pub(crate) fn renames(&self) -> &[Rename] {
        &self.renames
    }
    /// 打了包的文件操作请求和回执一共几字节；保留字节照样算它。
    pub(crate) fn file_bytes(&self) -> u64 {
        self.file_bytes
    }
    /// 这条路径有打了包的、已提交的正文：它一定有历史目录。
    pub(crate) fn committed(&self, path: &VaultPath) -> bool {
        self.committed.contains(path)
    }
    pub(crate) fn contains(&self, part: Part, name: &str) -> Result<bool, BrokerError> {
        Ok(!self.slots.is_empty() && self.slots.contains_key(&key(part, name)?))
    }
    /// 取一条原字节；取到的和索引里的摘要不一样就是坏的。
    pub(crate) fn read(
        &self,
        data: &Dir,
        part: Part,
        name: &str,
    ) -> Result<Option<Vec<u8>>, BrokerError> {
        if self.slots.is_empty() {
            return Ok(None);
        }
        let Some(slot) = self.slots.get(&key(part, name)?) else {
            return Ok(None);
        };
        let (file_name, length) = self
            .segments
            .get(slot.segment)
            .ok_or(BrokerError::CorruptWriterState)?;
        let bytes = read_range(data, file_name, *length, slot.offset, slot.bytes)?;
        if crate::types::digest(&bytes) != slot.sha256 {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(Some(bytes))
    }
    /// 把核过的一段接在链尾。段内、段间有一条重复就是坏的。
    pub(super) fn append(
        &mut self,
        name: String,
        length: u64,
        segment: &format::Segment,
        pair: VaultUuid,
    ) -> Result<(), BrokerError> {
        let bad = || BrokerError::CorruptWriterState;
        let (sequence, digest) = format::parse_name(&name).ok_or_else(bad)?;
        let previous = self.head.as_ref().map(|head| head.tail.as_str());
        let expected = self.head.as_ref().map_or(1, |head| head.segments + 1);
        if sequence != segment.sequence
            || sequence != expected
            || segment.previous.as_deref() != previous
        {
            return Err(bad());
        }
        let tail = digest.to_owned();
        let index = self.segments.len();
        for (pack, entries) in &segment.packs {
            for entry in entries {
                let slot = Slot {
                    segment: index,
                    offset: entry.offset,
                    bytes: entry.bytes,
                    sha256: key(Part::Replay, &entry.sha256)?.1,
                };
                if self
                    .slots
                    .insert(key(entry.part, &entry.name)?, slot)
                    .is_some()
                {
                    return Err(bad());
                }
            }
            self.summarize(pack, pair)?;
            self.packs.push(PackRef {
                kind: pack.kind,
                id: pack.id.clone(),
                key: pack.key.clone(),
            });
        }
        self.segments.push((name, length));
        self.head = Some(Head {
            segments: sequence,
            packs: self.packs.len() as u64,
            tail,
        });
        Ok(())
    }
    /// 一包里四条要说的是同一个请求；顺手记下别处判断要用的几样。
    fn summarize(&mut self, pack: &format::Pack, pair: VaultUuid) -> Result<(), BrokerError> {
        let bad = || BrokerError::CorruptWriterState;
        let [first, _, _, replay] = &pack.bytes;
        match (pack.kind, replay::decode(replay, Some(pair))?) {
            (Kind::Content, Replay::Current(terminal)) => {
                let envelope: recorded::store::Envelope = disk::decode(first)?;
                if envelope.id != pack.id
                    || envelope.key_hash != pack.key
                    || terminal.recording.as_ref() != Some(&envelope.reference()?)
                {
                    return Err(bad());
                }
                if matches!(terminal.outcome, Outcome::Committed { .. }) {
                    self.committed.insert(envelope.path);
                }
            }
            (Kind::Rename, Replay::FileOperation(terminal)) => {
                let request: file_operation::request::Request = disk::decode(first)?;
                let FileOperationKind::Rename { to, .. } = &request.frozen.kind else {
                    return Err(bad());
                };
                if request.frozen.operation_id != pack.id
                    || request.frozen.key_hash != pack.key
                    || terminal.frozen != request.frozen
                    || terminal.receipt.outcome != FileOperationOutcome::Committed
                {
                    return Err(bad());
                }
                self.renames.push(Rename {
                    source: request.frozen.path.clone(),
                    target: to.clone(),
                    source_revision: match &request.observed_before {
                        FilePresence::Present { revision, .. } => Some(*revision),
                        FilePresence::Missing => None,
                    },
                    target_revision: terminal.rename.as_ref().and_then(|r| r.target_revision),
                });
                for bytes in &pack.bytes[..3] {
                    self.file_bytes += bytes.len() as u64;
                }
            }
            _ => return Err(bad()),
        }
        Ok(())
    }
}

/// 列出 `archive-v1/` 里的段文件名；没有这个目录就是空。
pub(super) fn names(data: &Dir) -> Result<Option<Vec<String>>, BrokerError> {
    match data.symlink_metadata(DIRECTORY) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BrokerError::io("inspect writer archive", error)),
        Ok(_) => {}
    }
    let dir = directory(data, DIRECTORY)?;
    let mut names = Vec::new();
    for entry in dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate writer archive", e))?
    {
        let name = entry
            .map_err(|e| BrokerError::io("inspect writer archive", e))?
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        // 原子写断在改名之前的残留：开库清扫会挪走，没挪走的导出扫描会拒绝；它不是段。
        if is_exact_temporary_name(&name) {
            continue;
        }
        if format::parse_name(&name).is_none() {
            return Err(BrokerError::CorruptWriterState);
        }
        if names.len() >= 2 * MAX_SEGMENTS as usize {
            return Err(BrokerError::WriterLimitExceeded);
        }
        names.push(name);
    }
    names.sort();
    Ok(Some(names))
}

/// 整个读一段，文件名里的摘要要和内容一致。
pub(crate) fn read_segment(
    data: &Dir,
    name: &str,
    pair: VaultUuid,
    vault: VaultUuid,
) -> Result<(u64, format::Segment), BrokerError> {
    let (sequence, digest) = format::parse_name(name).ok_or(BrokerError::CorruptWriterState)?;
    let bytes = disk::read(data, &format!("{DIRECTORY}/{name}"), SEGMENT_BYTES)?
        .ok_or(BrokerError::CorruptWriterState)?;
    if sha256(&bytes) != digest {
        return Err(BrokerError::CorruptWriterState);
    }
    let segment = format::decode(&bytes, pair, vault)?;
    if segment.sequence != sequence {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok((bytes.len() as u64, segment))
}

fn read_range(
    data: &Dir,
    name: &str,
    length: u64,
    offset: u32,
    bytes: u32,
) -> Result<Vec<u8>, BrokerError> {
    let logical = VaultPath::parse(format!("{DIRECTORY}/{name}"))
        .map_err(|_| BrokerError::CorruptWriterState)?;
    let dir = directory(data, DIRECTORY)?;
    let mut file = FsVaultBroker::open_rooted_file(
        &dir,
        &VaultPath::parse(name).map_err(|_| BrokerError::CorruptWriterState)?,
    )
    .map_err(|error| match error {
        BrokerError::NotFound(_) => BrokerError::CorruptWriterState,
        other => other,
    })?;
    let metadata = file
        .metadata()
        .map_err(|e| BrokerError::io("stat writer archive segment", e))?;
    FsVaultBroker::validate_file_metadata(&logical, &metadata)?;
    if metadata.len() != length {
        return Err(BrokerError::CorruptWriterState);
    }
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(|e| BrokerError::io("seek writer archive segment", e))?;
    let mut value = vec![0u8; bytes as usize];
    file.read_exact(&mut value)
        .map_err(|e| BrokerError::io("read writer archive segment", e))?;
    Ok(value)
}

/// 从状态里的链尾往前找出整条链，再从头逐段核、逐段接。链上少一段、断一处都是坏的；不在链上的另外列出。
pub(super) fn load(
    data: &Dir,
    pair: VaultUuid,
    vault: VaultUuid,
    head: Option<&Head>,
) -> Result<Loaded, BrokerError> {
    let bad = || BrokerError::CorruptWriterState;
    let Some(mut names) = names(data)? else {
        return if head.is_some() {
            Err(bad())
        } else {
            Ok(Loaded {
                archive: Archive::default(),
                orphans: Vec::new(),
            })
        };
    };
    let mut chain = Vec::new();
    if let Some(head) = head {
        let mut digest = head.tail.clone();
        for sequence in (1..=head.segments).rev() {
            let name = format::file_name(sequence, &digest);
            let index = names.binary_search(&name).map_err(|_| bad())?;
            names.remove(index);
            if sequence > 1 {
                digest = format::previous(
                    &disk::read(data, &format!("{DIRECTORY}/{name}"), SEGMENT_BYTES)?
                        .ok_or_else(bad)?,
                )?;
            }
            chain.push(name);
        }
        chain.reverse();
    }
    let mut archive = Archive::default();
    for name in chain {
        let (length, segment) = read_segment(data, &name, pair, vault)?;
        archive.append(name, length, &segment, pair)?;
    }
    if archive.head() != head {
        return Err(bad());
    }
    Ok(Loaded {
        archive,
        orphans: names,
    })
}
