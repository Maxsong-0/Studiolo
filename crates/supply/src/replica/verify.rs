//! 发布清单与完整文件树的双次验证；任何遗漏、额外项或漂移都失败关闭。

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{ReadBatch, ReadEntryKind, ReadOnlyVault, ScanLimits};

use super::{
    Receipt, ReplicaError,
    protocol::{self, Manifest, ManifestEntry},
};
use crate::{Entry, EntryKind, Note, Snapshot};

pub(super) fn generation(
    root: &ReadOnlyVault,
    allow: &[VaultPath],
    receipt: &Receipt,
) -> Result<Snapshot, ReplicaError> {
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
    let expected = validate(&manifest, allow)?;
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
    Ok(first)
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate(
    manifest: &Manifest,
    allow: &[VaultPath],
) -> Result<BTreeMap<String, bool>, ReplicaError> {
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
    Ok(expected)
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
    let mut snapshot = Snapshot::default();
    for entry in &manifest.entries {
        append(vault, &batch, entry, &mut snapshot)?;
    }
    Ok(snapshot)
}

fn append(
    vault: &ReadOnlyVault,
    batch: &ReadBatch,
    entry: &ManifestEntry,
    snapshot: &mut Snapshot,
) -> Result<(), ReplicaError> {
    let path = protocol::visible(&entry.path)?;
    let kind = if entry.kind == "dir" {
        EntryKind::Directory
    } else if markdown(&entry.path) {
        // 笔记正文会对外供给：每次请求都整读并核 SHA-256，同大小字节漂移立即阻断。
        let data = vault
            .read_file(&path, protocol::FILE_LIMIT)
            .map_err(|_| ReplicaError::Drift("file_read_failed"))?;
        if data.content.len() as u64 != entry.bytes
            || Some(digest(&data.content)).as_ref() != entry.sha256.as_ref()
        {
            return Err(ReplicaError::Drift("file_digest"));
        }
        let index = batch
            .notes
            .binary_search_by(|note| note.path.cmp(&path))
            .map_err(|_| ReplicaError::Drift("missing_note"))?;
        let scanned = &batch.notes[index];
        if scanned.content != data.content {
            return Err(ReplicaError::Drift("note_changed_during_scan"));
        }
        snapshot.notes.push(Note {
            path: path.clone(),
            revision: data.revision,
            content: String::from_utf8(data.content)
                .map_err(|_| ReplicaError::Drift("non_utf8_note"))?,
        });
        EntryKind::Markdown
    } else {
        // 附件从不对外供给正文，只供给路径与大小（ADR-023）：每次请求只核同一次安全扫描
        // 得到的类型与字节长度，不整读、不算哈希。附件字节的完整性由接收端在副本到达时
        // 逐文件核 SHA-256 保证（scripts/replica_store.py）。整读附件曾让纳入 199 MB 的
        // 项目后每次请求多出约 1.5 秒。
        let index = batch
            .entries
            .binary_search_by(|scanned| scanned.path.cmp(&path))
            .map_err(|_| ReplicaError::Drift("missing_attachment"))?;
        let scanned = &batch.entries[index];
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
        let entries = (0..5)
            .map(|n| {
                serde_json::json!({
                    "path": format!("P/{n}.md"), "kind": "file", "bytes": 4 * 1024 * 1024,
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
            validate(&manifest, &allow),
            Err(ReplicaError::Invalid("manifest_notes_total_limit"))
        ));
        manifest.entries.pop();
        assert!(validate(&manifest, &allow).is_ok());
        // 附件总额与常驻正文预算分开；合法附件不挤占 Markdown 内存限额。
        manifest.entries.push(ManifestEntry {
            path: "P/attachment.bin".into(),
            kind: "file".into(),
            bytes: 16 * 1024 * 1024,
            sha256: Some("0".repeat(64)),
        });
        assert!(validate(&manifest, &allow).is_ok());
    }
}
