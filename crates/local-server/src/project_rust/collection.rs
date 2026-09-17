//! 两轮使用相同的已验证白名单；字节预算包括全部文件，失败不返回部分正文。

use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{BrokerError, ReadNote};
use studiolo_workbench::RustSourceInput;

use super::Failure;

const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_TOTAL_BYTES: usize = 512 * 1024;

pub(super) struct Captured {
    pub input: RustSourceInput,
    revision: Revision,
}

impl Captured {
    pub(super) fn same(&self, other: &Self) -> bool {
        self.input.path == other.input.path
            && self.input.content == other.input.content
            && self.revision == other.revision
    }
}

pub(super) fn capture(
    read: &mut impl FnMut(&VaultPath, u64) -> Result<ReadNote, BrokerError>,
    paths: &[VaultPath],
    verifying: bool,
) -> Result<Vec<Captured>, Failure> {
    let mut bytes = 0_usize;
    let mut result = Vec::with_capacity(paths.len());
    for path in paths {
        let remaining = MAX_TOTAL_BYTES.saturating_sub(bytes) as u64;
        let note = read(path, MAX_FILE_BYTES.min(remaining)).map_err(|_| {
            Failure::unavailable("Rust 源码缺失、不安全、不可读、超出读取预算或读取时变化。")
        })?;
        if note.path != *path || note.content.len() as u64 > MAX_FILE_BYTES {
            return Err(Failure::unavailable(
                "Rust 源码读取结果超出固定路径或单文件预算。",
            ));
        }
        bytes = bytes
            .checked_add(note.content.len())
            .filter(|bytes| *bytes <= MAX_TOTAL_BYTES)
            .ok_or_else(|| Failure::unavailable("Rust 源码超过每轮 512 KiB 正文预算。"))?;
        let content = String::from_utf8(note.content).map_err(|_| {
            if verifying {
                Failure::unavailable("Rust 源码在复核读取时变为无效 UTF-8，请重新读取。")
            } else {
                Failure::invalid("Rust 源码必须是有效 UTF-8 文本。")
            }
        })?;
        result.push(Captured {
            input: RustSourceInput {
                path: path.to_string(),
                content,
            },
            revision: note.revision,
        });
    }
    Ok(result)
}
