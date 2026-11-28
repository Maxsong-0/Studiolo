//! 段文件的固定编码：头里逐条写明类别、请求、长度和摘要，后面依次是原字节。头必须是唯一写法，重编一遍要一字不差。
use crate::{BrokerError, VaultUuid, sha256, writer::disk};
use serde::{Deserialize, Serialize};

pub(crate) const DIRECTORY: &str = "archive-v1";
const MAGIC: &[u8; 8] = b"SWARCH01";
/// 一段最多几包；也是写段的一步最多做的量，决定一步要持锁多久。
pub(crate) const SEGMENT_PACKS: usize = 64;
/// 一段最多几字节。一包最多四条、每条 64 KiB，64 包可以到 16 MiB，凑不下就少装几包。
pub(crate) const SEGMENT_BYTES: usize = 8 * 1024 * 1024;
/// 段头最多占这么多；挑的时候先留出来。
pub(crate) const HEADER_BYTES: usize = 256 * 1024;
/// 段数上限。能装多少包看每段装了多少（[`super::ArchiveBatch`]）：都装满是 65536 包，平时最少 32 包一段是 32768 包，
/// 一直在八成以上、每段只有 8 包是 8192 包；到 95% 起一包也写，一直在 95% 以上、每段只有 1 包的最坏情形只够 1024 包。
/// 离线命令每跑一次最多另外多一个小段。记录大到按字节装满的段可以少于 64 包（最少约 31 包）。
/// 再往上封存逐条核会先撞预算；离上限八成时健康页预警。
pub(crate) const MAX_SEGMENTS: u32 = 1024;

/// 写在 `writer-state.json` 里的段头：有几段、几包、链尾段的摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Head {
    pub segments: u32,
    pub packs: u64,
    pub tail: String,
}
impl Head {
    pub(crate) fn validate(&self) -> Result<(), BrokerError> {
        if self.segments == 0
            || self.segments > MAX_SEGMENTS
            || self.packs < u64::from(self.segments)
            || self.packs > u64::from(self.segments) * SEGMENT_PACKS as u64
            || !disk::hex(&self.tail)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(())
    }
}

/// 包的种类，决定里面有哪几条。后两种是 T-249 加的：段里有它们的，写者状态是第 10 版。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    /// 有审计主体的正文请求：描述符、两份回执、重放。
    Content,
    /// 已提交的改名：请求、两份回执、重放。
    Rename,
    /// 没有审计主体的正文请求（看板投影那种）：没有回执，只有描述符和重放。
    Unaudited,
    /// 已落定的别的文件操作：提交了的删除和恢复，取消或拒绝的任何一种。
    FileOperation,
}
impl Kind {
    pub(crate) fn parts(self) -> &'static [Part] {
        match self {
            Self::Content => &[
                Part::Descriptor,
                Part::IntentAck,
                Part::OutcomeAck,
                Part::Replay,
            ],
            Self::Rename | Self::FileOperation => &[
                Part::FileRequest,
                Part::FileIntentAck,
                Part::FileOutcomeAck,
                Part::Replay,
            ],
            Self::Unaudited => &[Part::Descriptor, Part::Replay],
        }
    }
    /// 第一条（散件在哪个目录、按哪个号记）。
    pub(crate) fn first(self) -> Part {
        self.parts()[0]
    }
    /// 正文请求的包（描述符打头）。
    pub(crate) fn content(self) -> bool {
        self.first() == Part::Descriptor
    }
    /// T-249 才有的种类：旧程序不认，段里有它就要第 10 版的写者状态。
    pub(crate) fn needs_v10(self) -> bool {
        matches!(self, Self::Unaudited | Self::FileOperation)
    }
}

/// 一条记录是哪一种。重放按幂等键摘要记，其余按请求号记。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Part {
    Descriptor,
    IntentAck,
    OutcomeAck,
    Replay,
    FileRequest,
    FileIntentAck,
    FileOutcomeAck,
}
impl Part {
    /// 散件放在哪个目录（相对写者数据根）。
    pub(crate) fn directory(self) -> &'static str {
        match self {
            Self::Descriptor | Self::IntentAck | Self::OutcomeAck => "history/meta-v1/requests",
            Self::Replay => "idempotency",
            Self::FileRequest => "file-operations-v1/records",
            Self::FileIntentAck | Self::FileOutcomeAck => "file-operations-v1/acks",
        }
    }
    /// 一条按什么记：重放按幂等键摘要，其余按请求号。
    pub(crate) fn name_of<'a>(self, id: &'a str, key: &'a str) -> &'a str {
        if self == Self::Replay { key } else { id }
    }
    /// 散件的文件名。
    pub(crate) fn file_name(self, id: &str) -> String {
        match self {
            Self::IntentAck => format!("{id}-intent.json.ack"),
            Self::OutcomeAck => format!("{id}-outcome.json.ack"),
            Self::FileIntentAck => format!("{id}-intent.json"),
            Self::FileOutcomeAck => format!("{id}-outcome.json"),
            Self::Descriptor | Self::Replay | Self::FileRequest => format!("{id}.json"),
        }
    }
}

/// 一包：请求号、幂等键摘要和各条原字节（按 [`Kind::parts`] 的顺序，条数和它一样）。
pub(crate) struct Pack {
    pub kind: Kind,
    pub id: String,
    pub key: String,
    pub bytes: Vec<Vec<u8>>,
}
impl Pack {
    /// 这一条按什么记。
    pub(crate) fn name_of(&self, part: Part) -> &str {
        part.name_of(&self.id, &self.key)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    pair_id: VaultUuid,
    vault_uuid: VaultUuid,
    sequence: u32,
    previous: Option<String>,
    packs: Vec<PackHeader>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackHeader {
    kind: Kind,
    id: String,
    key: String,
    /// 条数由种类定；编成 JSON 和原来的定长数组一字不差。
    entries: Vec<EntryHeader>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryHeader {
    part: Part,
    bytes: u32,
    sha256: String,
}

/// 段文件里的一条在哪、多长、摘要是什么。
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub part: Part,
    pub name: String,
    pub offset: u32,
    pub bytes: u32,
    pub sha256: String,
}
/// 解出来的一段。
pub(crate) struct Segment {
    pub sequence: u32,
    pub previous: Option<String>,
    pub packs: Vec<(Pack, Vec<Entry>)>,
}

/// 段文件的身份：第几段、接在谁后面、属于哪一对写者存储。
pub(crate) struct Place<'a> {
    pub pair: VaultUuid,
    pub vault: VaultUuid,
    pub sequence: u32,
    pub previous: Option<&'a str>,
}

/// 编成字节；太大（超过 [`SEGMENT_BYTES`]）就拒绝，调用方少装几包再来。
pub(crate) fn encode(place: &Place<'_>, packs: &[Pack]) -> Result<Vec<u8>, BrokerError> {
    if packs.is_empty() || packs.len() > SEGMENT_PACKS {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let mut payload = Vec::new();
    let mut headers = Vec::with_capacity(packs.len());
    for pack in packs {
        if pack.bytes.len() != pack.kind.parts().len() {
            return Err(BrokerError::CorruptWriterState);
        }
        let mut entries = Vec::with_capacity(pack.bytes.len());
        for (part, bytes) in pack.kind.parts().iter().copied().zip(&pack.bytes) {
            if bytes.is_empty() || bytes.len() > disk::RECORD_LIMIT {
                return Err(BrokerError::WriterLimitExceeded);
            }
            entries.push(EntryHeader {
                part,
                bytes: u32::try_from(bytes.len()).map_err(|_| BrokerError::WriterLimitExceeded)?,
                sha256: sha256(bytes),
            });
            payload.extend_from_slice(bytes);
        }
        headers.push(PackHeader {
            kind: pack.kind,
            id: pack.id.clone(),
            key: pack.key.clone(),
            entries,
        });
    }
    let header = serde_json::to_vec(&Header {
        version: 1,
        pair_id: place.pair,
        vault_uuid: place.vault,
        sequence: place.sequence,
        previous: place.previous.map(str::to_owned),
        packs: headers,
    })
    .map_err(|_| BrokerError::CorruptWriterState)?;
    let length = u32::try_from(header.len()).map_err(|_| BrokerError::WriterLimitExceeded)?;
    let mut output = Vec::with_capacity(12 + header.len() + payload.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&header);
    output.extend_from_slice(&payload);
    if output.len() > SEGMENT_BYTES {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(output)
}

/// 从字节里取前一段的摘要，用来沿链往前走；这一段对不对由之后整段核（[`decode`]）定。
pub(crate) fn previous(bytes: &[u8]) -> Result<String, BrokerError> {
    let (_, header) = header(bytes)?;
    disk::decode::<Header>(header)?
        .previous
        .ok_or(BrokerError::CorruptWriterState)
}

/// 头从哪开始、到哪结束。
fn header(bytes: &[u8]) -> Result<(usize, &[u8]), BrokerError> {
    let bad = || BrokerError::CorruptWriterState;
    if bytes.len() > SEGMENT_BYTES || bytes.get(..8) != Some(MAGIC.as_slice()) {
        return Err(bad());
    }
    let length: [u8; 4] = bytes
        .get(8..12)
        .and_then(|v| v.try_into().ok())
        .ok_or_else(bad)?;
    let end = 12usize
        .checked_add(u32::from_be_bytes(length) as usize)
        .ok_or_else(bad)?;
    Ok((end, bytes.get(12..end).ok_or_else(bad)?))
}

/// 解开并逐条核摘要；不属于这一对写者存储、不是唯一写法、多一个字节少一个字节都是坏的。
pub(crate) fn decode(
    bytes: &[u8],
    pair: VaultUuid,
    vault: VaultUuid,
) -> Result<Segment, BrokerError> {
    let bad = || BrokerError::CorruptWriterState;
    let (start, head) = header(bytes)?;
    let header: Header = disk::decode(head)?;
    if serde_json::to_vec(&header).map_err(|_| bad())? != head
        || header.version != 1
        || header.pair_id != pair
        || header.vault_uuid != vault
        || header.sequence == 0
        || header.sequence > MAX_SEGMENTS
        || (header.sequence == 1) != header.previous.is_none()
        || header.previous.as_deref().is_some_and(|v| !disk::hex(v))
        || header.packs.is_empty()
        || header.packs.len() > SEGMENT_PACKS
    {
        return Err(bad());
    }
    let mut offset = start;
    let mut packs = Vec::with_capacity(header.packs.len());
    for pack in header.packs {
        let parts = pack.kind.parts();
        if !disk::hex(&pack.id) || !disk::hex(&pack.key) || pack.entries.len() != parts.len() {
            return Err(bad());
        }
        let mut contents = Vec::with_capacity(parts.len());
        let mut entries = Vec::with_capacity(parts.len());
        for (expected, entry) in parts.iter().copied().zip(pack.entries) {
            let end = offset.checked_add(entry.bytes as usize).ok_or_else(bad)?;
            let content = bytes.get(offset..end).ok_or_else(bad)?;
            if entry.part != expected
                || content.is_empty()
                || content.len() > disk::RECORD_LIMIT
                || sha256(content) != entry.sha256
            {
                return Err(bad());
            }
            entries.push(Entry {
                part: entry.part,
                name: entry.part.name_of(&pack.id, &pack.key).to_owned(),
                offset: u32::try_from(offset).map_err(|_| bad())?,
                bytes: entry.bytes,
                sha256: entry.sha256,
            });
            contents.push(content.to_vec());
            offset = end;
        }
        packs.push((
            Pack {
                kind: pack.kind,
                id: pack.id,
                key: pack.key,
                bytes: contents,
            },
            entries,
        ));
    }
    if offset != bytes.len() {
        return Err(bad());
    }
    Ok(Segment {
        sequence: header.sequence,
        previous: header.previous,
        packs,
    })
}

/// 段文件名：8 位序号和全文件摘要。
pub(crate) fn file_name(sequence: u32, digest: &str) -> String {
    format!("{sequence:08}-{digest}.seg")
}
/// 认得出来的段文件名；别的一律不认。
pub(crate) fn parse_name(name: &str) -> Option<(u32, &str)> {
    let (number, digest) = name.strip_suffix(".seg")?.split_once('-')?;
    let sequence = number.parse().ok()?;
    (file_name(sequence, digest) == name && disk::hex(digest)).then_some((sequence, digest))
}
