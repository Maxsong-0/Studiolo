//! 整份封存的读回：`restic dump --archive tar` 的输出边读边核，正文不留（BUG-053）。
//!
//! 只认 restic（Go 的 archive/tar）会写的两种头：ustar 头和它前面可选的一个 PAX 扩展头；只认普通文件和目录。
//! 普通文件按清单核路径、字节数和 SHA-256，一份不多、一份不少、不重复；目录只许是封存根或清单里的目录，
//! 清单里的目录也一个不少（封存根自己 restic 不写，有没有都行）。
//! 结尾要有两个全零块，之后只许再跟全零字节。任何一处不对就记下，读完由 [`TarCheck::finish`] 报协议错误。
use super::{FILE_LIMIT, sink::Sink};
use crate::{
    ProcessError, Result,
    digest::{hex, is_sha256_hex},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const BLOCK: usize = 512;
const BLOCK_BYTES: u64 = BLOCK as u64;
/// PAX 扩展头正文的上限：restic 只在里面放路径、时间、属主名和扩展属性。
const PAX_LIMIT: u64 = 64 * 1024;
/// 结尾两个全零块之后还许跟的全零字节（按 tar 记录补齐的写法）。
const TAIL_LIMIT: u64 = 20 * BLOCK_BYTES;

/// 一份封存读回时该看到什么：封存根在 tar 里的名字、清单里的目录和文件。
pub(crate) struct TreeExpectation {
    /// 封存根的绝对路径去掉开头的 `/`：restic 写进 tar 的名字就是快照里的绝对路径去掉 `/`。
    root: String,
    directories: BTreeSet<String>,
    files: BTreeMap<String, (u64, String)>,
    /// 整条输出的上限：文件正文（按块补齐）加上每项最多一个 PAX 头、PAX 正文和一个 ustar 头，再加结尾。
    limit: u64,
}

impl TreeExpectation {
    pub fn new(root: &Path) -> Result<Self> {
        let name = root
            .to_str()
            .and_then(|text| text.strip_prefix('/'))
            .filter(|name| relative(name))
            .ok_or(ProcessError::Configuration)?;
        Ok(Self {
            root: name.to_owned(),
            directories: BTreeSet::new(),
            files: BTreeMap::new(),
            limit: 2 * BLOCK_BYTES + TAIL_LIMIT,
        })
    }
    /// 清单里的一个目录（相对封存根）。
    pub fn directory(&mut self, path: &str) -> Result<()> {
        if !relative(path) || !self.directories.insert(path.to_owned()) {
            return Err(ProcessError::Configuration);
        }
        self.grow(0)
    }
    /// 清单里的一个普通文件（相对封存根）和它的字节数、SHA-256。
    pub fn file(&mut self, path: &str, bytes: u64, sha256: &str) -> Result<()> {
        if !relative(path)
            || bytes > FILE_LIMIT
            || !is_sha256_hex(sha256)
            || self
                .files
                .insert(path.to_owned(), (bytes, sha256.to_owned()))
                .is_some()
        {
            return Err(ProcessError::Configuration);
        }
        self.grow(padded(bytes))
    }
    fn grow(&mut self, body: u64) -> Result<()> {
        self.limit = (2 * BLOCK_BYTES + padded(PAX_LIMIT))
            .checked_add(body)
            .and_then(|entry| self.limit.checked_add(entry))
            .ok_or(ProcessError::Configuration)?;
        Ok(())
    }
}

/// 读回时边收边核的接收器；见模块说明。
pub(super) struct TarCheck<'a> {
    expected: &'a TreeExpectation,
    remaining: BTreeMap<&'a str, (u64, &'a str)>,
    seen_directories: BTreeSet<String>,
    state: State<'a>,
    /// 还没凑满的头块，或正在收的 PAX 正文。
    held: Vec<u8>,
    /// 上一个 PAX 头给下一个头的改写。
    pending: Option<Pax>,
    failed: bool,
}

enum State<'a> {
    Header,
    /// 收 PAX 正文，收完再跳过补齐。
    Pax {
        left: u64,
        padding: u64,
    },
    /// 收一个文件的正文、算摘要，收完再跳过补齐。
    Data {
        left: u64,
        digest: Sha256,
        sha256: &'a str,
        padding: u64,
    },
    /// 跳过补齐的零字节。
    Skip {
        left: u64,
    },
    /// 看到第一个全零块，下一块也得全零。
    SecondZero,
    /// 两个全零块之后：只许再跟全零字节。
    End,
}

#[derive(Default)]
struct Pax {
    path: Option<String>,
    size: Option<u64>,
}

type Step = std::result::Result<usize, ()>;

impl<'a> TarCheck<'a> {
    pub fn new(expected: &'a TreeExpectation) -> Self {
        Self {
            expected,
            remaining: expected
                .files
                .iter()
                .map(|(path, (bytes, sha256))| (path.as_str(), (*bytes, sha256.as_str())))
                .collect(),
            seen_directories: BTreeSet::new(),
            state: State::Header,
            held: Vec::with_capacity(BLOCK),
            pending: None,
            failed: false,
        }
    }
    /// 输出读完之后：结构完整、清单里每个文件都核过、每个目录都见过，才算读回通过。
    pub fn finish(&self) -> Result<()> {
        // 见过的目录只会是封存根或清单里的、不重复（[`Self::directory`]），所以数够了就是一个不少。
        let directories = self
            .seen_directories
            .iter()
            .filter(|name| **name != self.expected.root)
            .count();
        if self.failed
            || !matches!(self.state, State::End)
            || !self.remaining.is_empty()
            || directories != self.expected.directories.len()
            || self.pending.is_some()
        {
            return Err(ProcessError::Protocol);
        }
        Ok(())
    }
    fn step(&mut self, bytes: &[u8]) -> Step {
        match &mut self.state {
            State::Header | State::SecondZero => {
                let take = (BLOCK - self.held.len()).min(bytes.len());
                self.held.extend_from_slice(&bytes[..take]);
                if self.held.len() == BLOCK {
                    let block: [u8; BLOCK] = self.held[..].try_into().map_err(|_| ())?;
                    self.held.clear();
                    self.header(&block)?;
                }
                Ok(take)
            }
            State::Pax { left, padding } => {
                let take = usize::try_from(*left).map_or(bytes.len(), |left| left.min(bytes.len()));
                self.held.extend_from_slice(&bytes[..take]);
                *left -= take as u64;
                if *left == 0 {
                    let padding = *padding;
                    let pax = parse_pax(&self.held)?;
                    self.held.clear();
                    self.pending = Some(pax);
                    self.state = skip(padding);
                }
                Ok(take)
            }
            State::Data {
                left,
                digest,
                sha256,
                padding,
            } => {
                let take = usize::try_from(*left).map_or(bytes.len(), |left| left.min(bytes.len()));
                digest.update(&bytes[..take]);
                *left -= take as u64;
                if *left == 0 {
                    let (sha256, padding) = (*sha256, *padding);
                    let State::Data { digest, .. } =
                        std::mem::replace(&mut self.state, skip(padding))
                    else {
                        return Err(());
                    };
                    if hex(&digest.finalize()) != sha256 {
                        return Err(());
                    }
                }
                Ok(take)
            }
            State::Skip { left } => {
                let take = usize::try_from(*left).map_or(bytes.len(), |left| left.min(bytes.len()));
                if bytes[..take].iter().any(|byte| *byte != 0) {
                    return Err(());
                }
                *left -= take as u64;
                if *left == 0 {
                    self.state = State::Header;
                }
                Ok(take)
            }
            State::End => {
                if bytes.iter().any(|byte| *byte != 0) {
                    return Err(());
                }
                Ok(bytes.len())
            }
        }
    }
    fn header(&mut self, block: &[u8; BLOCK]) -> std::result::Result<(), ()> {
        // 全零块之前挂着一个 PAX 头（它后面没有项）：结尾照常走，读完由 `finish` 看 `pending` 拒绝。
        if block.iter().all(|byte| *byte == 0) {
            self.state = match self.state {
                State::Header => State::SecondZero,
                _ => State::End,
            };
            return Ok(());
        }
        if matches!(self.state, State::SecondZero)
            || &block[257..263] != b"ustar\0"
            || &block[263..265] != b"00"
            || octal(&block[148..156]) != Some(checksum(block))
        {
            return Err(());
        }
        let stated = octal(&block[124..136]);
        match block[156] {
            b'x' => {
                let size = stated.filter(|size| *size <= PAX_LIMIT).ok_or(())?;
                if self.pending.is_some() {
                    return Err(());
                }
                self.state = if size == 0 {
                    self.pending = Some(Pax::default());
                    State::Header
                } else {
                    State::Pax {
                        left: size,
                        padding: padded(size) - size,
                    }
                };
                Ok(())
            }
            kind @ (b'0' | b'\0' | b'5') => {
                let pax = self.pending.take().unwrap_or_default();
                let name = match pax.path {
                    Some(path) => path,
                    None => ustar_name(block)?,
                };
                let size = pax.size.or(stated).ok_or(())?;
                if kind == b'5' {
                    self.directory(&name, size)
                } else {
                    self.file(&name, size)
                }
            }
            _ => Err(()),
        }
    }
    fn directory(&mut self, name: &str, size: u64) -> std::result::Result<(), ()> {
        let name = name.strip_suffix('/').unwrap_or(name);
        let known = name == self.expected.root
            || name
                .strip_prefix(self.expected.root.as_str())
                .and_then(|rest| rest.strip_prefix('/'))
                .is_some_and(|relative| self.expected.directories.contains(relative));
        if size != 0 || !known || !self.seen_directories.insert(name.to_owned()) {
            return Err(());
        }
        self.state = State::Header;
        Ok(())
    }
    fn file(&mut self, name: &str, size: u64) -> std::result::Result<(), ()> {
        let relative = name
            .strip_prefix(self.expected.root.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
            .ok_or(())?;
        let (bytes, sha256) = self.remaining.remove(relative).ok_or(())?;
        if size != bytes {
            return Err(());
        }
        let digest = Sha256::new();
        self.state = if size == 0 {
            if hex(&digest.finalize()) != sha256 {
                return Err(());
            }
            State::Header
        } else {
            State::Data {
                left: size,
                digest,
                sha256,
                padding: padded(size) - size,
            }
        };
        Ok(())
    }
}

impl Sink for TarCheck<'_> {
    fn limit(&self) -> u64 {
        self.expected.limit
    }
    fn append(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() && !self.failed {
            match self.step(bytes) {
                Ok(used) => bytes = &bytes[used..],
                Err(()) => self.failed = true,
            }
        }
    }
}

fn skip<'a>(padding: u64) -> State<'a> {
    if padding == 0 {
        State::Header
    } else {
        State::Skip { left: padding }
    }
}

/// 相对路径：非空，不以 `/` 开头或结尾，没有空段、`.`、`..`。
fn relative(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn padded(bytes: u64) -> u64 {
    bytes.div_ceil(BLOCK_BYTES) * BLOCK_BYTES
}

/// 头块校验和：校验和字段按 8 个空格算的无符号字节和（Go 写的就是这一种）。
fn checksum(block: &[u8; BLOCK]) -> u64 {
    block
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum()
}

/// 八进制数字段：前后可有空格和 NUL，中间只许八进制数字；base-256 写法不认（需要时 restic 会写 PAX 记录）。
fn octal(field: &[u8]) -> Option<u64> {
    let text = std::str::from_utf8(field)
        .ok()?
        .trim_matches(|c: char| c == ' ' || c == '\0');
    if text.is_empty() || !text.bytes().all(|byte| (b'0'..=b'7').contains(&byte)) {
        return None;
    }
    u64::from_str_radix(text, 8).ok()
}

/// ustar 的名字：前缀字段非空时是「前缀/名字」。
fn ustar_name(block: &[u8; BLOCK]) -> std::result::Result<String, ()> {
    let field = |range: std::ops::Range<usize>| {
        let raw = &block[range];
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        std::str::from_utf8(&raw[..end]).map_err(|_| ())
    };
    let (name, prefix) = (field(0..100)?, field(345..500)?);
    if name.is_empty() {
        return Err(());
    }
    Ok(if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    })
}

/// PAX 正文：一串「长度 键=值\n」记录。只取 path 和 size：path 要是 UTF-8，size 只许十进制数字；
/// 键为空、同一个键出现两次、稀疏文件和链接名的记录都不认。别的记录（时间、属主名、扩展属性 `SCHILY.xattr.*`）
/// 只核记录本身的格式、不看值：Go 的 archive/tar 只禁这些键里有 NUL，扩展属性的值是原样字节，可以不是 UTF-8。
fn parse_pax(body: &[u8]) -> std::result::Result<Pax, ()> {
    let mut pax = Pax::default();
    let mut rest = body;
    while !rest.is_empty() {
        let space = rest.iter().position(|byte| *byte == b' ').ok_or(())?;
        let length = decimal(&rest[..space])
            .and_then(|length| usize::try_from(length).ok())
            .filter(|length| *length > space + 1 && *length <= rest.len())
            .ok_or(())?;
        let record = &rest[space + 1..length];
        let (record, newline) = record.split_at(record.len() - 1);
        if newline != b"\n" {
            return Err(());
        }
        let equals = record.iter().position(|byte| *byte == b'=').ok_or(())?;
        let (key, value) = (&record[..equals], &record[equals + 1..]);
        match key {
            b"" => return Err(()),
            b"path" if pax.path.is_none() => {
                pax.path = Some(std::str::from_utf8(value).map_err(|_| ())?.to_owned());
            }
            b"size" if pax.size.is_none() => pax.size = Some(decimal(value).ok_or(())?),
            b"path" | b"size" => return Err(()),
            key if key.starts_with(b"GNU.sparse") || key == b"linkpath" => return Err(()),
            _ => {}
        }
        rest = &rest[length..];
    }
    Ok(pax)
}

/// 十进制数字串：非空，只有 ASCII 数字（不带符号、空格）。
fn decimal(digits: &[u8]) -> Option<u64> {
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

#[cfg(test)]
#[path = "../../tests/support/tar_builder.rs"]
pub(crate) mod builder;
#[cfg(test)]
#[path = "../../tests/support/process_tar.rs"]
mod tests;
