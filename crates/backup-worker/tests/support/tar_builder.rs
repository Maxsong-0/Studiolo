//! 测试用：照 Go archive/tar（restic dump 用的）的写法拼 tar 流。名字是 ASCII、放得下就只写 ustar 头
//! （超过 100 字节时按 `/` 拆进前缀字段）；不是 ASCII 或拆不开就在前面加一个 PAX 头写 `path`。
use super::{TarCheck, TreeExpectation};
use crate::{Result, process::sink::Sink};
use sha2::{Digest, Sha256};

pub(crate) const REGULAR: u8 = b'0';
pub(crate) const DIRECTORY: u8 = b'5';

/// 一个 ustar 头块；`name` 原样写（调用方保证放得下），校验和照 Go 的写法。
pub(crate) fn header(name: &str, kind: u8, size: u64) -> [u8; 512] {
    let mut block = [0_u8; 512];
    let (prefix, base) = split(name).unwrap_or(("", name));
    block[..base.len()].copy_from_slice(base.as_bytes());
    octal(&mut block[100..108], 0o440);
    octal(&mut block[108..116], 1000);
    octal(&mut block[116..124], 1000);
    octal(&mut block[124..136], size);
    octal(&mut block[136..148], 1_700_000_000);
    block[156] = kind;
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");
    block[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    seal(&mut block);
    block
}

/// 改过头块之后重算校验和。
pub(crate) fn seal(block: &mut [u8; 512]) {
    block[148..156].copy_from_slice(b"        ");
    let sum: u64 = block.iter().map(|byte| u64::from(*byte)).sum();
    block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
}

fn octal(field: &mut [u8], value: u64) {
    let text = format!("{value:0width$o}\0", width = field.len() - 1);
    field.copy_from_slice(text.as_bytes());
}

/// ustar 放得下的拆法：前缀 ≤155 字节、名字 ≤100 字节，都是 ASCII。
fn split(name: &str) -> Option<(&str, &str)> {
    if !name.is_ascii() {
        return None;
    }
    if name.len() <= 100 {
        return Some(("", name));
    }
    name.match_indices('/')
        .map(|(at, _)| (&name[..at], &name[at + 1..]))
        .find(|(prefix, base)| prefix.len() <= 155 && !base.is_empty() && base.len() <= 100)
}

/// 一条 PAX 记录「长度 键=值\n」，长度算上它自己的位数；键和值是原样字节（扩展属性的值可以不是 UTF-8）。
pub(crate) fn record(key: &[u8], value: &[u8]) -> Vec<u8> {
    let base = key.len() + value.len() + 3;
    let mut length = base + base.to_string().len();
    if length.to_string().len() > base.to_string().len() {
        length += 1;
    }
    let mut out = format!("{length} ").into_bytes();
    out.extend_from_slice(key);
    out.push(b'=');
    out.extend_from_slice(value);
    out.push(b'\n');
    out
}

/// 一个 PAX 扩展头（头块加补齐过的正文）。
pub(crate) fn pax(records: &[(&str, &str)]) -> Vec<u8> {
    let body = records
        .iter()
        .flat_map(|(key, value)| record(key.as_bytes(), value.as_bytes()))
        .collect::<Vec<u8>>();
    pax_body(&body)
}

/// 一个 PAX 扩展头，正文原样给（可以故意写坏）。
pub(crate) fn pax_body(body: &[u8]) -> Vec<u8> {
    let mut out = header("PaxHeaders.0/entry", b'x', body.len() as u64).to_vec();
    out.extend_from_slice(body);
    pad(&mut out);
    out
}

/// 改一项里的 ustar 头（不是 PAX 头的那一块），改完重算校验和。
pub(crate) fn retouch(mut entry: Vec<u8>, change: impl Fn(&mut [u8; 512])) -> Vec<u8> {
    let at = entry
        .chunks(512)
        .position(|block| block[156] != b'x' && &block[257..263] == b"ustar\0")
        .unwrap()
        * 512;
    let block: &mut [u8; 512] = (&mut entry[at..at + 512]).try_into().unwrap();
    change(block);
    seal(block);
    entry
}

/// 一项：要 PAX 就先写 PAX 头，ustar 头里放一个截短的 ASCII 名字（Go 也是这样）。
pub(crate) fn entry(name: &str, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let shown = if split(name).is_some() {
        name.to_owned()
    } else {
        out.extend(pax(&[("path", name), ("mtime", "1700000000.5")]));
        name.chars()
            .filter(char::is_ascii)
            .take(99)
            .collect::<String>()
            + "_"
    };
    out.extend_from_slice(&header(&shown, kind, body.len() as u64));
    out.extend_from_slice(body);
    pad(&mut out);
    out
}

pub(crate) fn file(name: &str, body: &[u8]) -> Vec<u8> {
    entry(name, REGULAR, body)
}

/// 目录项：restic 写的名字带结尾的 `/`。
pub(crate) fn directory(name: &str) -> Vec<u8> {
    entry(&format!("{name}/"), DIRECTORY, b"")
}

/// 归档结尾：两个全零块。
pub(crate) fn end() -> Vec<u8> {
    vec![0; 1024]
}

fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().div_ceil(512) * 512, 0);
}

pub(crate) fn sha(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 把整条流按 `chunk` 字节一段喂给接收器，再看读完的结论；也核整条流没超出上限。
pub(crate) fn check(expected: &TreeExpectation, stream: &[u8], chunk: usize) -> Result<()> {
    let mut sink = TarCheck::new(expected);
    if stream.len() as u64 > sink.limit() {
        return Err(crate::ProcessError::OutputLimit);
    }
    for piece in stream.chunks(chunk) {
        sink.append(piece);
    }
    sink.finish()
}
