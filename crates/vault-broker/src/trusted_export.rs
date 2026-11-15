//! 读取 owner 导出目录并给出来源事实。尝试正文只承认清单第 2 版；未知目录失败关闭。
use crate::BrokerError;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    Revision, RevisionHasher, TrustedExportClass, TrustedExportFact, VaultPath, VaultUuid,
};

const MAX_FILES: usize = 100_000;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const CATALOG_LIMIT: u64 = 8 * 1024 * 1024;

/// 扫描一次导出根。缺来源的尝试文件、孤立历史 blob 和未知顶层目录都拒绝。
pub fn read_trusted_export(
    export_root: &Path,
    vault: VaultUuid,
) -> Result<Vec<TrustedExportFact>, BrokerError> {
    let mut files = BTreeMap::new();
    walk(export_root, export_root, &vault, &mut files, 0)?;
    let mut facts = BTreeMap::new();
    let mut logical = BTreeMap::new();
    register_attempts(export_root, &files, &mut facts, &mut logical)?;
    register_owner_files(&files, &mut facts, &mut logical)?;
    register_history(export_root, &files, &logical, &mut facts)?;
    if facts.len() != files.len() {
        return Err(rejected());
    }
    Ok(facts.into_values().collect())
}

fn register_attempts(
    export_root: &Path,
    files: &BTreeMap<VaultPath, Revision>,
    facts: &mut BTreeMap<VaultPath, TrustedExportFact>,
    logical: &mut BTreeMap<VaultPath, DataLabels>,
) -> Result<(), BrokerError> {
    let mut listed = BTreeSet::new();
    for path in files.keys() {
        if !is_attempt_manifest(path) {
            continue;
        }
        let bytes = read_limited(&export_root.join(path.as_str()), MANIFEST_LIMIT)?;
        let manifest: ManifestView = serde_json::from_slice(&bytes).map_err(|_| rejected())?;
        let provenance = match (manifest.format_version, manifest.provenance) {
            (2, Some(provenance)) => provenance,
            _ => return Err(old_manifest()),
        };
        let labels = provenance.labels().clone();
        listed.insert(path.clone());
        facts.insert(
            path.clone(),
            fact(
                path,
                files,
                TrustedExportClass::SourceMaterial,
                Some(provenance.clone()),
            )?,
        );
        if let Some(vault_path) = path.as_str().strip_prefix("canonical/") {
            logical.insert(
                VaultPath::parse(vault_path).map_err(|_| rejected())?,
                labels.clone(),
            );
        }
        for artifact in manifest.artifacts {
            let export =
                VaultPath::parse(format!("canonical/{}", artifact.path)).map_err(|_| rejected())?;
            if !files.contains_key(&export) || listed.contains(&export) {
                return Err(rejected());
            }
            listed.insert(export.clone());
            facts.insert(
                export.clone(),
                fact(
                    &export,
                    files,
                    TrustedExportClass::SourceMaterial,
                    Some(provenance.clone()),
                )?,
            );
            logical.insert(artifact.path, labels.clone());
        }
    }
    if files
        .keys()
        .any(|path| is_attempt_file(path) && !listed.contains(path))
    {
        return Err(rejected());
    }
    Ok(())
}

fn register_owner_files(
    files: &BTreeMap<VaultPath, Revision>,
    facts: &mut BTreeMap<VaultPath, TrustedExportFact>,
    logical: &mut BTreeMap<VaultPath, DataLabels>,
) -> Result<(), BrokerError> {
    let labels = owner_labels()?;
    for path in files.keys() {
        if facts.contains_key(path) || is_history_blob(path) {
            continue;
        }
        let class = owner_class(path).ok_or_else(rejected)?;
        facts.insert(path.clone(), fact(path, files, class, None)?);
        if let Some(vault_path) = path.as_str().strip_prefix("canonical/") {
            logical.insert(
                VaultPath::parse(vault_path).map_err(|_| rejected())?,
                labels.clone(),
            );
        }
    }
    Ok(())
}

fn register_history(
    export_root: &Path,
    files: &BTreeMap<VaultPath, Revision>,
    logical: &BTreeMap<VaultPath, DataLabels>,
    facts: &mut BTreeMap<VaultPath, TrustedExportFact>,
) -> Result<(), BrokerError> {
    let mut blobs = BTreeMap::new();
    for path in files.keys().filter(|path| is_history_catalog(path)) {
        let bytes = read_limited(&export_root.join(path.as_str()), CATALOG_LIMIT)?;
        let catalog: HistoryCatalog = serde_json::from_slice(&bytes).map_err(|_| rejected())?;
        if !matches!(catalog.version, 1..=5) {
            return Err(rejected());
        }
        let _ = (catalog.garbage, catalog.feed_retire, catalog.pair_id);
        for item in catalog.entries {
            let revision = item.entry.get("revision").ok_or_else(rejected)?;
            let revision: Revision =
                serde_json::from_value(revision.clone()).map_err(|_| rejected())?;
            let export = VaultPath::parse(format!("writer-data/history/{revision}"))
                .map_err(|_| rejected())?;
            if blobs.insert(export, catalog.path.clone()).is_some() {
                return Err(rejected());
            }
        }
    }
    for path in files.keys().filter(|path| is_history_blob(path)) {
        let source_path = blobs.get(path).ok_or_else(rejected)?;
        let labels = logical.get(source_path).ok_or_else(rejected)?;
        if labels.backup_scope() != BackupScope::Full {
            return Err(rejected());
        }
        let provenance = facts
            .get(&VaultPath::parse(format!("canonical/{source_path}")).map_err(|_| rejected())?)
            .and_then(|fact| fact.provenance.clone());
        facts.insert(
            path.clone(),
            fact(path, files, TrustedExportClass::FullScopeL0, provenance)?,
        );
    }
    Ok(())
}

fn fact(
    path: &VaultPath,
    files: &BTreeMap<VaultPath, Revision>,
    class: TrustedExportClass,
    provenance: Option<ArtifactProvenance>,
) -> Result<TrustedExportFact, BrokerError> {
    Ok(TrustedExportFact {
        export_path: path.clone(),
        class,
        provenance,
        revision: *files.get(path).ok_or_else(rejected)?,
    })
}

fn owner_labels() -> Result<DataLabels, BrokerError> {
    DataLabels::new(
        Classification::Restricted,
        BackupScope::Full,
        DesktopSync::Deny,
        ModelEgress::Deny,
    )
    .map_err(|_| rejected())
}

fn owner_class(path: &VaultPath) -> Option<TrustedExportClass> {
    match path.as_str().split('/').next()? {
        "control" => Some(TrustedExportClass::ControlDatabase),
        "audit" | "agent-audit" => Some(TrustedExportClass::AuditLedger),
        "canonical" | "writer-data" | "auth" | "bridge" | "transport" | "plane" | "agent"
        | "policy" => Some(TrustedExportClass::Canonical),
        _ => None,
    }
}

fn is_attempt_file(path: &VaultPath) -> bool {
    path.as_str().starts_with("canonical/sources/attempts/")
}
fn is_attempt_manifest(path: &VaultPath) -> bool {
    is_attempt_file(path) && path.as_str().ends_with("/manifest.json")
}
fn is_history_blob(path: &VaultPath) -> bool {
    let mut parts = path.as_str().split('/');
    parts.next() == Some("writer-data")
        && parts.next() == Some("history")
        && !matches!(parts.next(), Some("meta-v1") | None)
}
fn is_history_catalog(path: &VaultPath) -> bool {
    path.as_str()
        .starts_with("writer-data/history/meta-v1/catalogs/")
        && path.as_str().ends_with(".json")
}

fn walk(
    export_root: &Path,
    directory: &Path,
    vault: &VaultUuid,
    files: &mut BTreeMap<VaultPath, Revision>,
    depth: u32,
) -> Result<(), BrokerError> {
    if depth > 64 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let entries = fs::read_dir(directory).map_err(|error| BrokerError::io("read export", error))?;
    for entry in entries {
        let entry = entry.map_err(|error| BrokerError::io("read export entry", error))?;
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| BrokerError::io("stat export", error))?;
        if metadata.file_type().is_symlink() {
            return Err(rejected());
        }
        if metadata.is_dir() {
            walk(export_root, &entry.path(), vault, files, depth + 1)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(rejected());
        }
        if files.len() >= MAX_FILES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let absolute = entry.path();
        let relative = absolute.strip_prefix(export_root).map_err(|_| rejected())?;
        let path = VaultPath::parse(relative.to_string_lossy()).map_err(|_| rejected())?;
        files.insert(path.clone(), hash_file(&absolute, *vault, &path)?);
    }
    Ok(())
}

fn hash_file(path: &Path, vault: VaultUuid, logical: &VaultPath) -> Result<Revision, BrokerError> {
    let file = fs::File::open(path).map_err(|error| BrokerError::io("read export file", error))?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = RevisionHasher::new(vault, logical);
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = std::io::Read::read(&mut reader, &mut buffer)
            .map_err(|error| BrokerError::io("hash export file", error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finish())
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, BrokerError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| BrokerError::io("stat export file", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err(rejected());
    }
    fs::read(path).map_err(|error| BrokerError::io("read export file", error))
}

fn rejected() -> BrokerError {
    BrokerError::InvalidArtifact("导出文件没有可接受的来源登记".into())
}
fn old_manifest() -> BrokerError {
    BrokerError::InvalidArtifact("旧清单缺少来源元数据，不能升为完整范围".into())
}

#[derive(Deserialize)]
struct ManifestView {
    format_version: u32,
    #[serde(default)]
    provenance: Option<ArtifactProvenance>,
    #[serde(default)]
    artifacts: Vec<ArtifactView>,
}
#[derive(Deserialize)]
struct ArtifactView {
    path: VaultPath,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCatalog {
    version: u32,
    path: VaultPath,
    entries: Vec<HistoryItem>,
    #[serde(default)]
    garbage: Vec<Revision>,
    #[serde(default)]
    feed_retire: Vec<(u64, String)>,
    #[allow(dead_code)]
    pair_id: VaultUuid,
}
#[derive(Deserialize)]
struct HistoryItem {
    entry: serde_json::Value,
}
