//! 发布清单与完整文件树的双次验证；任何遗漏、额外项或漂移都失败关闭。

use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;
use studiolo_vault_broker::{ReadEntry, ReadEntryKind, ReadNote, ReadOnlyVault, ScanLimits};

use super::{
    Receipt, ReplicaError, Usage,
    protocol::{self, Manifest, ManifestEntry},
};
use crate::{Entry, EntryKind, Note, Snapshot};

pub(super) fn generation(
    root: &ReadOnlyVault,
    allow: &[VaultPath],
    receipt: &Receipt,
) -> Result<(Snapshot, Usage), ReplicaError> {
    let release_path = VaultPath::parse(format!("releases/{}", receipt.generation))
        .map_err(|_| ReplicaError::Invalid("generation_path"))?;
    let release = root
        .open_directory(&release_path)
        .map_err(|_| ReplicaError::Drift("unreadable_generation"))?;
    let manifest_path = VaultPath::parse("manifest.json").expect("fixed path");
    let raw = release
        .read_file(&manifest_path, protocol::MANIFEST_LIMIT)
        .map_err(|_| ReplicaError::Invalid("unreadable_manifest"))?
        .content;
    if digest(&raw) != receipt.source_digest {
        return Err(ReplicaError::Invalid("manifest_digest"));
    }
    let manifest: Manifest =
        serde_json::from_slice(&raw).map_err(|_| ReplicaError::Invalid("manifest_json"))?;
    let (expected, usage) = validate(&manifest, allow, raw.len() as u64)?;
    let vault = release
        .open_directory(&VaultPath::parse("vault").expect("fixed path"))
        .map_err(|_| ReplicaError::Drift("unreadable_vault"))?;
    let first = capture(&vault, &manifest, &expected)?;
    let second = capture(&vault, &manifest, &expected)?;
    if first != second {
        return Err(ReplicaError::Drift("capture_changed"));
    }
    let after = release
        .read_file(&manifest_path, protocol::MANIFEST_LIMIT)
        .map_err(|_| ReplicaError::Invalid("unreadable_manifest"))?;
    if raw != after.content {
        return Err(ReplicaError::Drift("manifest_changed"));
    }
    Ok((first, usage))
}

fn digest(bytes: &[u8]) -> String {
    studiolo_core::sha256_hex(bytes)
}

/// 核清单的形状与各条上限；顺带交出这一代用了各条上限多少（`manifest_bytes` 是清单原始字节数），供路牌提前提醒。
fn validate(
    manifest: &Manifest,
    allow: &[VaultPath],
    manifest_bytes: u64,
) -> Result<(BTreeMap<String, bool>, Usage), ReplicaError> {
    let allowed = allow.iter().map(|path| path.as_str()).collect::<Vec<_>>();
    if manifest.schema_version != 1
        || manifest
            .allow
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != allowed
    {
        return Err(ReplicaError::Invalid("manifest_allowlist"));
    }
    if manifest.entries.len() > protocol::ENTRY_LIMIT {
        return Err(ReplicaError::Invalid("manifest_entry_limit"));
    }
    let mut expected = BTreeMap::new();
    for prefix in allow {
        let mut parts = String::new();
        for part in prefix.as_str().split('/') {
            if !parts.is_empty() {
                parts.push('/');
            }
            parts.push_str(part);
            expected.insert(parts.clone(), true);
        }
    }
    let mut previous: Option<&str> = None;
    let mut total = 0u64;
    let mut notes_total = 0u64;
    for entry in &manifest.entries {
        protocol::visible(&entry.path)?;
        if previous.is_some_and(|old| old >= entry.path.as_str())
            || !allowed
                .iter()
                .any(|prefix| entry.path.starts_with(&format!("{prefix}/")))
        {
            return Err(ReplicaError::Invalid("manifest_order_or_scope"));
        }
        previous = Some(&entry.path);
        let directory = match entry.kind.as_str() {
            "dir" if entry.bytes == 0 && entry.sha256.is_none() => true,
            "file"
                if entry.bytes <= protocol::FILE_LIMIT
                    && entry
                        .sha256
                        .as_deref()
                        .is_some_and(|hash| protocol::hex(hash, 64)) =>
            {
                false
            }
            _ => return Err(ReplicaError::Invalid("manifest_entry_type")),
        };
        if !directory && markdown(&entry.path) && entry.bytes > protocol::NOTE_LIMIT {
            return Err(ReplicaError::Invalid("manifest_note_limit"));
        }
        if !directory && markdown(&entry.path) {
            notes_total = notes_total
                .checked_add(entry.bytes)
                .filter(|total| *total <= protocol::NOTES_TOTAL_LIMIT)
                .ok_or(ReplicaError::Invalid("manifest_notes_total_limit"))?;
        }
        total = total
            .checked_add(entry.bytes)
            .filter(|total| *total <= protocol::TOTAL_LIMIT)
            .ok_or(ReplicaError::Invalid("manifest_total_limit"))?;
        if expected.insert(entry.path.clone(), directory).is_some() {
            return Err(ReplicaError::Invalid("duplicate_manifest_path"));
        }
    }
    let mut folded = BTreeSet::new();
    for path in expected.keys() {
        if !folded.insert(path.to_lowercase()) {
            return Err(ReplicaError::Invalid("case_collision"));
        }
        if let Some((parent, _)) = path.rsplit_once('/')
            && expected.get(parent) != Some(&true)
        {
            return Err(ReplicaError::Invalid("missing_manifest_parent"));
        }
    }
    let usage = Usage {
        manifest_bytes,
        entries: manifest.entries.len(),
        total_bytes: total,
        notes_bytes: notes_total,
    };
    Ok((expected, usage))
}

fn capture(
    vault: &ReadOnlyVault,
    manifest: &Manifest,
    expected: &BTreeMap<String, bool>,
) -> Result<Snapshot, ReplicaError> {
    let batch = vault
        .scan(ScanLimits {
            max_file_bytes: protocol::NOTE_LIMIT,
            max_total_bytes: protocol::NOTES_TOTAL_LIMIT,
            max_entries: protocol::ENTRY_LIMIT + 4096,
            max_depth: 64,
        })
        .map_err(|_| ReplicaError::Drift("scan_failed"))?;
    if !batch.diagnostics.is_empty() || batch.entries.len() != expected.len() {
        return Err(ReplicaError::Drift("incomplete_or_extra_entries"));
    }
    for entry in &batch.entries {
        if expected.get(entry.path.as_str()) != Some(&(entry.kind == ReadEntryKind::Directory)) {
            return Err(ReplicaError::Drift("tree_mismatch"));
        }
    }
    // 扫描结果按路径排好序，清单也严格按路径递增（`validate` 核过），所以两边的 Markdown 一一对应、
    // 顺着走一遍即可；每篇扫描读到的正文只交出一次，快照直接接手，不再复制一份。
    let mut notes = batch.notes.into_iter();
    let mut snapshot = Snapshot::default();
    for entry in &manifest.entries {
        append(&batch.entries, &mut notes, entry, &mut snapshot)?;
    }
    if notes.next().is_some() {
        return Err(ReplicaError::Drift("extra_note"));
    }
    Ok(snapshot)
}

fn append(
    scanned_entries: &[ReadEntry],
    scanned_notes: &mut impl Iterator<Item = ReadNote>,
    entry: &ManifestEntry,
    snapshot: &mut Snapshot,
) -> Result<(), ReplicaError> {
    let path = protocol::visible(&entry.path)?;
    let kind = if entry.kind == "dir" {
        EntryKind::Directory
    } else if markdown(&entry.path) {
        // 笔记正文会对外供给：每次请求都整读并核 SHA-256，同大小字节漂移立即阻断。
        // 正文用的是同一次安全扫描读到的字节。扫描已把每层目录的名字枚举一遍、查过大小写撞名，
        // 并按不跟随链接、不跨设备、私有权限、读前读后元数据不变的规矩读了每一篇；之后整轮捕获再做一遍、
        // 两轮逐字节比对。不再按路径单独重读（T-215，vault BUG-039）：那条路每读一篇都从根重新解析路径，
        // 把沿途每层目录的名字整个枚举两遍，一次校验的耗时是「每篇所在目录的条目数」之和，
        // 同一目录篇数一多就成了平方级（108 实测 4000 篇挤在一个目录时单次校验 23.8 秒，其中 94% 在这里）。
        let note = scanned_notes
            .next()
            .filter(|note| note.path == path)
            .ok_or(ReplicaError::Drift("missing_note"))?;
        if note.content.len() as u64 != entry.bytes
            || Some(digest(&note.content)).as_ref() != entry.sha256.as_ref()
        {
            return Err(ReplicaError::Drift("file_digest"));
        }
        snapshot.notes.push(Note {
            path: path.clone(),
            revision: note.revision,
            content: String::from_utf8(note.content)
                .map_err(|_| ReplicaError::Drift("non_utf8_note"))?
                .into(),
        });
        EntryKind::Markdown
    } else {
        // 附件从不对外供给正文，只供给路径与大小（ADR-023）：每次请求只核同一次安全扫描
        // 得到的类型与字节长度，不整读、不算哈希。附件字节的完整性由接收端在副本到达时
        // 逐文件核 SHA-256 保证（scripts/replica_store.py）。整读附件曾让纳入 199 MB 的
        // 项目后每次请求多出约 1.5 秒。
        let index = scanned_entries
            .binary_search_by(|scanned| scanned.path.cmp(&path))
            .map_err(|_| ReplicaError::Drift("missing_attachment"))?;
        let scanned = &scanned_entries[index];
        if scanned.kind != ReadEntryKind::Attachment || scanned.size_bytes != Some(entry.bytes) {
            return Err(ReplicaError::Drift("attachment_size"));
        }
        EntryKind::Attachment
    };
    snapshot.entries.push(Entry {
        path,
        kind,
        size_bytes: (kind != EntryKind::Directory).then_some(entry.bytes),
    });
    Ok(())
}

fn markdown(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialized_manifest_caps_retained_markdown_before_reading_bodies() {
        // 篇篇顶到单篇上限、合计刚好超出总量上限一篇；弹掉一篇就刚好等于上限。按常量推，上限调整时不必改这里。
        // 文件名补零：清单要求路径按字节序严格递增，`P/10.md` 排在 `P/9.md` 前面，篇数一到两位数
        // 就会先撞上顺序检查（T-215 上限调到 40 MiB、11 篇时踩到）。
        const { assert!(protocol::NOTES_TOTAL_LIMIT.is_multiple_of(protocol::NOTE_LIMIT)) };
        let entries = (0..=protocol::NOTES_TOTAL_LIMIT / protocol::NOTE_LIMIT)
            .map(|n| {
                serde_json::json!({
                    "path": format!("P/{n:05}.md"), "kind": "file", "bytes": protocol::NOTE_LIMIT,
                    "sha256": "0".repeat(64)
                })
            })
            .collect::<Vec<_>>();
        let mut manifest: Manifest = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "allow": ["P"], "entries": entries
        }))
        .unwrap();
        let allow = vec![VaultPath::parse("P").unwrap()];
        assert!(matches!(
            validate(&manifest, &allow, 0),
            Err(ReplicaError::Invalid("manifest_notes_total_limit"))
        ));
        manifest.entries.pop();
        let (_, usage) = validate(&manifest, &allow, 123).unwrap();
        let notes = manifest.entries.len();
        // 用量与越线判定同一组数：刚好等于上限时 Markdown 合计就是上限。
        assert_eq!(
            usage,
            super::Usage {
                manifest_bytes: 123,
                entries: notes,
                total_bytes: protocol::NOTES_TOTAL_LIMIT,
                notes_bytes: protocol::NOTES_TOTAL_LIMIT,
            }
        );
        // 附件总额与常驻正文预算分开；合法附件不挤占 Markdown 内存限额，只算进文件总量和条目数。
        manifest.entries.push(ManifestEntry {
            path: "P/attachment.bin".into(),
            kind: "file".into(),
            bytes: 16 * 1024 * 1024,
            sha256: Some("0".repeat(64)),
        });
        let (_, usage) = validate(&manifest, &allow, 123).unwrap();
        assert_eq!(usage.notes_bytes, protocol::NOTES_TOTAL_LIMIT);
        assert_eq!(
            usage.total_bytes,
            protocol::NOTES_TOTAL_LIMIT + 16 * 1024 * 1024
        );
        assert_eq!(usage.entries, notes + 1);
    }
}
