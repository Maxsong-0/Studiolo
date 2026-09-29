//! 观察证据的纯投影：`Revision` 与 Bridge 摘要的互换、范围选择投影、路径类别与批准用证据；无 IO、无时钟。
//!
//! Bridge 侧的 `FileVersion::revision` 直接承载 `Revision` 的 32 字节原值（ADR-025 2026-09-26 补充）：
//! 条件写／条件删除的前置在任何一轮都能按证据还原，重开核对不再依赖「观察它的那一轮」。

use super::super::PlaneError;
use std::collections::BTreeMap;
use studiolo_core::{Revision, VaultPath};
use studiolo_host_config::{
    SyncAttachmentType, SyncContentMetadata, SyncFileKind, SyncObservationSelection,
    SyncPathEvidence, SyncTraversalRule,
};
use studiolo_sync_bridge::{Baseline, Digest, Inventory, Presence, Side};
use studiolo_vault_broker::{SyncReadRule, SyncReadSelection};

/// `Revision` 在 Bridge 证据里的固定投影：它的 32 字节原值。
#[must_use]
pub fn revision_digest(revision: Revision) -> Digest {
    revision.to_bytes()
}

/// 从 Bridge 证据里的摘要还原原 `Revision`；与 [`revision_digest`] 互逆。
#[must_use]
pub fn revision_from_digest(digest: &Digest) -> Revision {
    Revision::from_bytes(*digest)
}

/// 域内通用 SHA-256。
#[must_use]
pub fn sha256(bytes: &[u8]) -> Digest {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).into()
}

/// 解析 64 位小写十六进制摘要；长度或字符不对返回 `None`。
#[must_use]
pub fn parse_digest(hex: &str) -> Option<Digest> {
    if hex.len() != 64 {
        return None;
    }
    let mut output = [0u8; 32];
    for (index, slot) in output.iter_mut().enumerate() {
        *slot = u8::from_str_radix(hex.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(output)
}

/// 小写十六进制。
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

/// 把范围的观察选择投影成 Broker 的读取选择；`max_bytes` 逐规则照抄。
#[must_use]
pub fn read_selection(selection: &SyncObservationSelection) -> SyncReadSelection {
    let rule = |rule: &SyncTraversalRule| SyncReadRule {
        path: rule.path().clone(),
        max_bytes: rule.policy().max_bytes(),
    };
    SyncReadSelection {
        folders: selection.folders().iter().map(rule).collect(),
        files: selection.files().iter().map(rule).collect(),
        excluded_prefixes: selection.excluded_prefixes().to_vec(),
    }
}

/// 范围内所有规则的最大对象字节数（至少 1）；驱动器与 `sync-init` 都用它冻结 `Limits`。
#[must_use]
pub fn max_object_bytes(selection: &SyncObservationSelection) -> u64 {
    selection
        .folders()
        .iter()
        .chain(selection.files())
        .map(|rule| rule.policy().max_bytes())
        .max()
        .unwrap_or(1)
        .max(1)
}

/// 路径的内容类别：扩展名决定，`.md` 还须 canonical 侧证明是完整 UTF-8；
/// 没登记的扩展名不可批准，返回 `None`。
#[must_use]
pub fn content_kind(path: &VaultPath, utf8_valid: Option<bool>) -> Option<SyncFileKind> {
    let name = path.as_str().rsplit('/').next().unwrap_or_default();
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    let attachment = |kind| Some(SyncFileKind::Attachment(kind));
    match extension.as_deref() {
        Some("md" | "markdown") => (utf8_valid != Some(false)).then_some(SyncFileKind::Markdown),
        Some("png") => attachment(SyncAttachmentType::Png),
        Some("jpg" | "jpeg") => attachment(SyncAttachmentType::Jpeg),
        Some("gif") => attachment(SyncAttachmentType::Gif),
        Some("webp") => attachment(SyncAttachmentType::Webp),
        Some("pdf") => attachment(SyncAttachmentType::Pdf),
        Some("mp3") => attachment(SyncAttachmentType::Mp3),
        Some("mp4") => attachment(SyncAttachmentType::Mp4),
        Some("wav") => attachment(SyncAttachmentType::Wav),
        Some("zip") => attachment(SyncAttachmentType::Zip),
        Some("txt") => attachment(SyncAttachmentType::Txt),
        Some("json") => attachment(SyncAttachmentType::Json),
        _ => None,
    }
}

/// 一侧观察的批准用证据；canonical 侧带 UTF-8 事实，镜像侧只看扩展名。
///
/// # Errors
/// 出现不可批准的类别返回 `Observation`。
pub fn inventory_evidence(
    inventory: &Inventory,
    utf8: &BTreeMap<VaultPath, bool>,
) -> Result<Vec<SyncPathEvidence>, PlaneError> {
    inventory
        .entries
        .iter()
        .map(|(path, presence)| {
            let content = match presence {
                Presence::Missing => None,
                Presence::Present(version) => {
                    let utf8 = (inventory.side == Side::Canonical)
                        .then(|| utf8.get(path).copied())
                        .flatten();
                    Some(SyncContentMetadata {
                        kind: content_kind(path, utf8).ok_or(PlaneError::Observation)?,
                        bytes: version.bytes,
                    })
                }
            };
            Ok(SyncPathEvidence {
                path: path.clone(),
                content,
            })
        })
        .collect()
}

/// 基线证据：每项双方同内容，取 canonical 侧（缺失时取镜像侧）的字节数与路径类别。
///
/// # Errors
/// 出现不可批准的类别返回 `Observation`。
pub fn baseline_evidence(baseline: &Baseline) -> Result<Vec<SyncPathEvidence>, PlaneError> {
    baseline
        .entries
        .iter()
        .map(|(path, pair)| {
            let version = match (&pair.canonical, &pair.mirror) {
                (Presence::Present(version), _) | (_, Presence::Present(version)) => Some(version),
                (Presence::Missing, Presence::Missing) => None,
            };
            let content = version
                .map(|version| {
                    Ok(SyncContentMetadata {
                        kind: content_kind(path, None).ok_or(PlaneError::Observation)?,
                        bytes: version.bytes,
                    })
                })
                .transpose()?;
            Ok(SyncPathEvidence {
                path: path.clone(),
                content,
            })
        })
        .collect()
}
