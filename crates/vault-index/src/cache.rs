//! 单篇派生解析的纯编码；持久存储、来源验证和授权由调用方负责。
//!
//! 摘要用于绑定精确输入，不认证派生语义。恢复前必须验证私有存储，
//! 缓存不能决定可见路径，也不能恢复正文、身份、新鲜度或跨文档链接结论。

use std::{collections::BTreeMap, io};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use studiolo_core::VaultPath;

use crate::{
    InMemoryVaultIndex, IndexError, IndexGeneration, MarkdownDocument, parse::ParsedDocument,
};

/// 序列化缓存的硬字节预算；超限不影响正常索引构建。
pub const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
/// 单文解析规则版本；Markdown、YAML 正规化语义变化时必须递增。
pub const PARSE_CACHE_VERSION: u32 = 2;
const SCHEMA_VERSION: u32 = 1;
const MAX_DOCUMENTS: usize = 25_000;

/// 本次构建实际复用和重新解析的篇数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseCacheStats {
    /// 路径、字节数和正文 SHA-256 均匹配的篇数。
    pub reused: usize,
    /// 本次重新进行 YAML/Markdown 解析的篇数。
    pub parsed: usize,
}

/// 缓存可丢弃；这些错误都允许调用方回退到正常构建。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    /// 编码或解码超过固定缓存预算。
    #[error("解析缓存超过预算")]
    TooLarge,
    /// JSON 损坏、结构不符或存在未知字段。
    #[error("解析缓存编码无效")]
    InvalidEncoding,
    /// 旧结构或解析规则版本不适用于当前程序。
    #[error("解析缓存版本不匹配")]
    UnsupportedVersion,
    /// 缓存行重复、乱序或字段不符合派生结构约束。
    #[error("解析缓存条目无效")]
    InvalidRecord,
}

/// 可序列化的单文解析集合，不含输入 Markdown 正文。
#[derive(Clone, Debug)]
pub struct ParseCache {
    data: CacheFile,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CacheFile {
    schema_version: u32,
    parser_version: u32,
    entries: Vec<CachedDocument>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedDocument {
    path: VaultPath,
    content_sha256: String,
    content_bytes: usize,
    parsed: ParsedDocument,
}

impl ParseCache {
    /// 仅解码有界字节；不读取文件，也不把编码摘要当作授权证据。
    pub fn decode(raw: &[u8]) -> Result<Self, CacheError> {
        if raw.len() > MAX_CACHE_BYTES {
            return Err(CacheError::TooLarge);
        }
        let cache = Self {
            data: serde_json::from_slice(raw).map_err(|_| CacheError::InvalidEncoding)?,
        };
        cache.validate()?;
        Ok(cache)
    }

    /// 有界编码；超限时丢弃输出，已构建的内存模型仍然有效。
    pub fn encode(&self) -> Result<Vec<u8>, CacheError> {
        self.validate()?;
        let mut writer = BoundedWriter::default();
        if serde_json::to_writer(&mut writer, &self.data).is_err() {
            return Err(if writer.exceeded {
                CacheError::TooLarge
            } else {
                CacheError::InvalidEncoding
            });
        }
        Ok(writer.bytes)
    }

    fn validate(&self) -> Result<(), CacheError> {
        if self.data.schema_version != SCHEMA_VERSION
            || self.data.parser_version != PARSE_CACHE_VERSION
        {
            return Err(CacheError::UnsupportedVersion);
        }
        if self.data.entries.len() > MAX_DOCUMENTS {
            return Err(CacheError::TooLarge);
        }
        let mut previous = None;
        for entry in &self.data.entries {
            if previous.is_some_and(|path| path >= &entry.path)
                || entry.content_sha256.len() != 64
                || !entry
                    .content_sha256
                    .bytes()
                    .all(|ch| matches!(ch, b'0'..=b'9' | b'a'..=b'f'))
                || !entry.valid_locations()
            {
                return Err(CacheError::InvalidRecord);
            }
            previous = Some(&entry.path);
        }
        Ok(())
    }

    fn find(&self, path: &VaultPath, hash: &str, bytes: usize) -> Option<&ParsedDocument> {
        let row = &self.data.entries[self
            .data
            .entries
            .binary_search_by(|row| row.path.cmp(path))
            .ok()?];
        (row.content_sha256 == hash && row.content_bytes == bytes).then_some(&row.parsed)
    }
}

impl CachedDocument {
    fn valid_locations(&self) -> bool {
        let max_line = self.content_bytes.saturating_add(1);
        let mut line = 0;
        for heading in &self.parsed.headings {
            if !(1..=6).contains(&heading.level)
                || heading.line <= line
                || heading.line as usize > max_line
            {
                return false;
            }
            line = heading.line;
        }
        self.parsed.links.iter().all(|link| {
            link.line > 0
                && link.line as usize <= max_line
                && link.column > 0
                && link.column as usize <= max_line
        })
    }
}

impl InMemoryVaultIndex {
    /// 当前输入决定全部路径；仅匹配单文解析可复用，全局关系每次重新派生。
    pub fn rebuild_cached(
        &mut self,
        documents: &[MarkdownDocument],
        previous: Option<&ParseCache>,
    ) -> Result<(IndexGeneration, ParseCache, ParseCacheStats), IndexError> {
        let mut replacement = BTreeMap::new();
        let mut entries = Vec::new();
        let mut stats = ParseCacheStats::default();
        for input in documents {
            if replacement.contains_key(input.path()) {
                return Err(IndexError::DuplicatePath(input.path().clone()));
            }
            let hash = digest(input.contents().as_bytes());
            let bytes = input.contents().len();
            let parsed = previous
                .and_then(|cache| cache.find(input.path(), &hash, bytes))
                .map_or_else(
                    || {
                        stats.parsed += 1;
                        ParsedDocument::parse(input)
                    },
                    |parsed| {
                        stats.reused += 1;
                        parsed.clone()
                    },
                );
            replacement.insert(
                input.path().clone(),
                parsed
                    .clone()
                    .finish(input.path(), input.unavailable_reason()),
            );
            entries.push(CachedDocument {
                path: input.path().clone(),
                content_sha256: hash,
                content_bytes: bytes,
                parsed,
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let cache = ParseCache {
            data: CacheFile {
                schema_version: SCHEMA_VERSION,
                parser_version: PARSE_CACHE_VERSION,
                entries,
            },
        };
        Ok((self.publish(replacement), cache, stats))
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Default)]
struct BoundedWriter {
    bytes: Vec<u8>,
    exceeded: bool,
}

impl io::Write for BoundedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > MAX_CACHE_BYTES.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("解析缓存超过预算"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;
