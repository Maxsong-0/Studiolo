//! 保全索引 `index.json` 的封闭文档、纯编解码与条目运算，外加只认已有描述符的字节助手；
//! 本文件不打开任何路径。
//!
//! 版本固定 1，`checksum` 是对全部条目的域分隔 sha256；撕裂、未知字段、乱序、重复、
//! 超限一律失败关闭，绝不把半写洗成「空索引」。

use super::{INDEX_MAX_BYTES, RETENTION_SECONDS};
use crate::sync_plane::PlaneError;
use rustix::fd::BorrowedFd;
use rustix::io::Errno;
use sha2::{Digest, Sha256};

const VERSION: u8 = 1;
const DOMAIN: &[u8] = b"studiolo.sync-plane.stage-index.v1\0";

/// 保全索引里的一条：谁保全的、保全时刻、是否仍被引用。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreservedEntry {
    /// 对象地址（sha256 小写十六进制）。
    pub object: String,
    /// 对象字节数。
    pub bytes: u64,
    /// 保全时刻（Unix 秒）。
    pub preserved_at_unix: i64,
    /// 产生保全的计划序号；退休后可回收。
    pub plan_sequence: u64,
    /// 原逻辑路径（只用于人工找回，不参与任何判定）。
    pub original_path: String,
}

/// 固定名索引文档：版本 1，带校验和，失败关闭。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageIndex {
    /// 固定为 1。
    pub version: u8,
    /// 保全条目；按对象地址排序且不重复。
    pub preserved: Vec<PreservedEntry>,
    /// 对上面字段的域分隔 sha256（小写十六进制）。
    pub checksum: String,
}

impl StageIndex {
    /// 空索引：索引文件不存在或管理员初始化时的起点。
    #[must_use]
    pub fn empty() -> Self {
        let preserved = Vec::new();
        let checksum = checksum(&preserved);
        Self {
            version: VERSION,
            preserved,
            checksum,
        }
    }

    /// 编码成持久字节；超过预算即失败，不截断。
    pub(super) fn encode(&self) -> Result<Vec<u8>, PlaneError> {
        let bytes = serde_json::to_vec(self).map_err(|_| PlaneError::Storage)?;
        if bytes.len() > INDEX_MAX_BYTES {
            return Err(PlaneError::Storage);
        }
        Ok(bytes)
    }

    /// 解码持久字节：未知字段、版本不符、乱序或重复地址、校验和不符一律失败关闭。
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, PlaneError> {
        if bytes.len() > INDEX_MAX_BYTES {
            return Err(PlaneError::Storage);
        }
        let document: Self = serde_json::from_slice(bytes).map_err(|_| PlaneError::Storage)?;
        let ordered = document
            .preserved
            .windows(2)
            .all(|pair| pair[0].object < pair[1].object);
        if document.version != VERSION
            || !ordered
            || document.checksum != checksum(&document.preserved)
        {
            return Err(PlaneError::Storage);
        }
        Ok(document)
    }

    /// 是否已登记该地址。
    #[must_use]
    pub fn contains(&self, object: &str) -> bool {
        self.position(object).is_ok()
    }

    fn position(&self, object: &str) -> Result<usize, usize> {
        self.preserved
            .binary_search_by(|entry| entry.object.as_str().cmp(object))
    }

    /// 登记或刷新一条保全；同一地址再次保全以最新一次为准，保留期从最新时刻重算。
    pub(super) fn upsert(&mut self, entry: PreservedEntry) {
        match self.position(&entry.object) {
            Ok(index) => self.preserved[index] = entry,
            Err(index) => self.preserved.insert(index, entry),
        }
        self.checksum = checksum(&self.preserved);
    }

    /// 取走保留期已到且计划已退休的条目地址；索引与校验和随之更新。
    pub(super) fn take_expired(&mut self, now_unix: i64, retired_through: u64) -> Vec<String> {
        let (expired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.preserved)
            .into_iter()
            .partition(|entry| {
                entry.plan_sequence <= retired_through
                    && now_unix
                        .checked_sub(entry.preserved_at_unix)
                        .and_then(|age| u64::try_from(age).ok())
                        .is_some_and(|age| age >= RETENTION_SECONDS)
            });
        self.preserved = kept;
        self.checksum = checksum(&self.preserved);
        expired.into_iter().map(|entry| entry.object).collect()
    }
}

/// 小写十六进制。
#[must_use]
pub fn hex(bytes: [u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn checksum(preserved: &[PreservedEntry]) -> String {
    let mut digest = Sha256::new();
    digest.update(DOMAIN);
    digest.update([VERSION]);
    digest.update((preserved.len() as u64).to_be_bytes());
    for entry in preserved {
        digest.update((entry.object.len() as u64).to_be_bytes());
        digest.update(entry.object.as_bytes());
        digest.update(entry.bytes.to_be_bytes());
        digest.update(entry.preserved_at_unix.to_be_bytes());
        digest.update(entry.plan_sequence.to_be_bytes());
        digest.update((entry.original_path.len() as u64).to_be_bytes());
        digest.update(entry.original_path.as_bytes());
    }
    hex(digest.finalize().into())
}

/// 读满约定长度，再多读一字节确认 EOF；读取期间被改写的证据不接受。
pub(in crate::sync_plane) fn read_fully(
    file: BorrowedFd<'_>,
    length: usize,
) -> rustix::io::Result<Vec<u8>> {
    let mut bytes = vec![0_u8; length];
    let mut tail = [0_u8; 1];
    let mut filled = 0;
    loop {
        let target: &mut [u8] = if filled < length {
            &mut bytes[filled..]
        } else {
            &mut tail
        };
        match rustix::io::read(file, target) {
            Ok(0) if filled == length => return Ok(bytes),
            Ok(0) => return Err(Errno::IO),
            Ok(_) if filled == length => return Err(Errno::IO),
            Ok(count) => filled += count,
            Err(Errno::INTR) => {}
            Err(error) => return Err(error),
        }
    }
}

/// 写满全部字节。
pub(in crate::sync_plane) fn write_fully(
    file: BorrowedFd<'_>,
    bytes: &[u8],
) -> rustix::io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        match rustix::io::write(file, &bytes[written..]) {
            Ok(0) => return Err(Errno::IO),
            Ok(count) => written += count,
            Err(Errno::INTR) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
