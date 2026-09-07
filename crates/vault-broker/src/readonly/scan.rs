//! 有界的单次枚举：文件树和 Markdown 来自同一次扫描，但不是文件系统事务快照。

use cap_std::fs::{Dir, FileType};
use std::collections::BTreeMap;
use studiolo_core::{VaultPath, VaultUuid};

use super::{
    ReadBatch, ReadEntry, ReadEntryKind, ReadOnlyVault, ScanLimits, limit, metadata, rooted,
};
use crate::BrokerError;

const MAX_PATH_BYTES: usize = 16 * 1024 * 1024;

impl ReadOnlyVault {
    /// 一次扫描返回可见目录、Markdown 内容和普通附件元信息，不读取附件字节。
    ///
    /// 目录包括空目录、纯附件目录；根目录不另造逻辑路径。Markdown 的单文件/总
    /// 字节限额保持 ScanLimits 语义，附件长度不消耗内容预算。所有枚举项（包括隐藏项）
    /// 消耗条目预算；可见路径累计最多 16 MiB，非法名称和诊断也不能绕过该上限。
    /// 单项失败会诊断不完整；目录变动或任何总量限额失败会返回 Err，而非截断结果。
    /// 每项/目录经过局部稳定性检查，但不保证扫描结束前其他已扫描文件不会再变化。
    pub fn scan(&self, limits: ScanLimits) -> Result<ReadBatch, BrokerError> {
        self.scan_in_namespace(limits, super::read::replica_namespace())
    }

    pub(super) fn scan_in_namespace(
        &self,
        limits: ScanLimits,
        namespace: VaultUuid,
    ) -> Result<ReadBatch, BrokerError> {
        let mut scan = ScanState {
            limits,
            namespace,
            inspected: 0,
            bytes: 0,
            path_bytes: 0,
            batch: ReadBatch::default(),
        };
        self.scan_directory(&self.root, "", 0, &mut scan)?;
        scan.batch
            .entries
            .sort_by(|left, right| left.path.cmp(&right.path));
        scan.batch
            .notes
            .sort_by(|left, right| left.path.cmp(&right.path));
        scan.batch.diagnostics.sort();
        Ok(scan.batch)
    }

    /// 兼容入口，委托统一扫描；notes 保持原义，entries 新增完整可见文件树。
    /// 不安全的普通附件同样会产生不完整诊断，不再静默遗漏。
    pub fn scan_markdown(&self, limits: ScanLimits) -> Result<ReadBatch, BrokerError> {
        self.scan(limits)
    }

    fn scan_directory(
        &self,
        directory: &Dir,
        prefix: &str,
        depth: usize,
        scan: &mut ScanState,
    ) -> Result<(), BrokerError> {
        let before = directory
            .dir_metadata()
            .map_err(|error| BrokerError::io("stat scan directory", error))?;
        let (entries, names) = enumerate(directory, prefix, scan)?;
        for (name, path, kind) in entries {
            if names[&name.to_lowercase()] != 1 {
                scan.diagnostic(&path, "case-colliding filesystem name skipped");
            } else if kind.is_symlink() {
                scan.diagnostic(&path, "symbolic link skipped");
            } else if kind.is_dir() {
                if depth >= scan.limits.max_depth {
                    return Err(limit("scan depth limit exceeded"));
                }
                let child = match rooted::scanned_directory(&self.root, directory, &name, &path) {
                    Ok(child) => child,
                    Err(_) => {
                        scan.diagnostic(&path, "unsafe or unreadable directory skipped");
                        continue;
                    }
                };
                let opened = child
                    .dir_metadata()
                    .map_err(|error| BrokerError::io("stat opened scan directory", error))?;
                self.scan_directory(&child, path.as_str(), depth + 1, scan)?;
                // 重新从选定根解析该名称，不能只检查已经被移出原位置的旧句柄。
                let current = rooted::scanned_directory(&self.root, directory, &name, &path)?
                    .dir_metadata()
                    .map_err(|error| BrokerError::io("recheck scan directory identity", error))?;
                if !rooted::unchanged(&opened, &current) {
                    return Err(BrokerError::UnstableRead(path));
                }
                scan.batch.entries.push(ReadEntry {
                    path,
                    kind: ReadEntryKind::Directory,
                    size_bytes: None,
                });
            } else if !kind.is_file() {
                scan.diagnostic(&path, "non-regular entry skipped");
            } else if name
                .rsplit_once('.')
                .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
            {
                match self.read_scanned(
                    directory,
                    &name,
                    &path,
                    scan.limits.max_file_bytes,
                    scan.namespace,
                ) {
                    Ok(note) => {
                        let size = note.content.len() as u64;
                        scan.bytes = scan
                            .bytes
                            .checked_add(size)
                            .filter(|bytes| *bytes <= scan.limits.max_total_bytes)
                            .ok_or_else(|| limit("scan total bytes limit exceeded"))?;
                        scan.batch.entries.push(ReadEntry {
                            path,
                            kind: ReadEntryKind::Markdown,
                            size_bytes: Some(size),
                        });
                        scan.batch.notes.push(note);
                    }
                    Err(_) => scan.diagnostic(
                        &path,
                        "unsafe, changed, unreadable or oversized file skipped",
                    ),
                }
            } else {
                match metadata::stable_file(&self.root, directory, &name, &path) {
                    Ok(size) => scan.batch.entries.push(ReadEntry {
                        path,
                        kind: ReadEntryKind::Attachment,
                        size_bytes: Some(size),
                    }),
                    Err(_) => {
                        scan.diagnostic(&path, "unsafe, changed or unreadable attachment skipped")
                    }
                }
            }
        }
        let after = directory
            .dir_metadata()
            .map_err(|error| BrokerError::io("recheck scan directory", error))?;
        if !rooted::unchanged(&before, &after) {
            return Err(limit("source directory changed during scan"));
        }
        Ok(())
    }
}

type Entries = Vec<(String, VaultPath, FileType)>;

// 同层只枚举一次；名称索引仅在本次扫描内有效，独立 read_file 不使用这项信任。
fn enumerate(
    directory: &Dir,
    prefix: &str,
    scan: &mut ScanState,
) -> Result<(Entries, BTreeMap<String, usize>), BrokerError> {
    let mut entries = Vec::new();
    let mut names = BTreeMap::new();
    for entry in directory
        .entries()
        .map_err(|error| BrokerError::io("enumerate scan directory", error))?
    {
        let entry = entry.map_err(|error| BrokerError::io("read scan entry", error))?;
        scan.inspected = scan
            .inspected
            .checked_add(1)
            .filter(|count| *count <= scan.limits.max_entries)
            .ok_or_else(|| limit("scan entry limit exceeded"))?;
        let name = entry.file_name();
        if name.as_encoded_bytes().starts_with(b".") {
            continue;
        }
        let raw_len = prefix
            .len()
            .saturating_add(usize::from(!prefix.is_empty()))
            .saturating_add(name.as_encoded_bytes().len());
        scan.path_bytes = scan
            .path_bytes
            .checked_add(raw_len)
            .filter(|bytes| *bytes <= MAX_PATH_BYTES)
            .ok_or_else(|| limit("scan logical path bytes limit exceeded"))?;
        let Some(name) = name.to_str() else {
            scan.batch
                .diagnostics
                .push(format!("{prefix}: non-UTF-8 entry skipped"));
            continue;
        };
        *names.entry(name.to_lowercase()).or_insert(0) += 1;
        let raw = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let path = match VaultPath::parse(raw) {
            Ok(path) => path,
            Err(_) => {
                scan.batch
                    .diagnostics
                    .push(format!("{prefix}: noncanonical entry skipped"));
                continue;
            }
        };
        match entry.file_type() {
            Ok(kind) => entries.push((name.to_owned(), path, kind)),
            Err(_) => scan.diagnostic(&path, "entry metadata unavailable"),
        }
    }
    Ok((entries, names))
}

struct ScanState {
    limits: ScanLimits,
    namespace: VaultUuid,
    inspected: usize,
    bytes: u64,
    path_bytes: usize,
    batch: ReadBatch,
}

impl ScanState {
    fn diagnostic(&mut self, path: &VaultPath, reason: &str) {
        self.batch.diagnostics.push(format!("{path}: {reason}"));
    }
}
