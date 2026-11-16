//! 扫描 owner 导出目录，给每个文件一条来源事实（T-204）。
//!
//! 分类只凭两样东西：固定的 owner 顶层目录，和 Broker 自己写下的清单与历史目录。不按文件名猜标签。
//!
//! - 尝试目录（`canonical/sources/attempts/<任务>/<尝试>/`）里的文件，只承认本目录清单第 2 版
//!   列出的，来源就是清单里的原批准。旧清单、没被清单列出的文件，整批拒绝。
//! - 历史正文跟着它所属的那篇文档走：文档是来源资料，旧版本也是来源资料（同一份原批准、
//!   同一个期限）；文档是普通 canonical 内容（哪怕已经删掉），旧版本是完整范围 L0。
//!   没有目录认领的历史正文拒绝。
//! - 暂存正文（`writer-data/artifact-candidate-<id>.json`）装着还没发布、或被黑板留作候选的产物全文，
//!   它是来源资料，带它自己那份原批准和期限。按顶层目录归成普通写者状态的话，期限对它就不起作用了。
//!   升级前留下、没有原批准的暂存读不出来，整批拒绝。
//! - 原子写留下的临时文件（`.<名字>.studiolo-tmp-…`）不管在哪一层都拒绝：它可能是任何文件的半截或全文。
//! - 其余文件按顶层目录归 owner 声明的类别；不认识的顶层目录拒绝。
//!
//! 本模块按路径读文件，自己不防并发替换：导出根是封存窗口里刚写出的私有目录，
//! 封存时还会按这里算出的 revision 逐个文件复验，中途被换掉的文件过不了那一关。
use crate::{BrokerError, sha256};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Revision, RevisionHasher, TrustedExportClass,
    TrustedExportFact, VaultPath, VaultUuid,
};

const MAX_FILES: usize = 100_000;
const MAX_DEPTH: u32 = 64;
const MANIFEST_LIMIT: u64 = 1024 * 1024;
const CATALOG_LIMIT: u64 = 8 * 1024 * 1024;
const CANONICAL: &str = "canonical/";
const ATTEMPTS: &str = "sources/attempts/";
const HISTORY: &str = "writer-data/history/";
const HISTORY_METADATA: &str = "meta-v1";
const HISTORY_CATALOGS: &str = "writer-data/history/meta-v1/catalogs/";
const MANIFEST_NAME: &str = "manifest.json";
const WRITER_DATA: &str = "writer-data/";
/// 暂存正文的上限是 8 MiB（`artifact_stage::LIMIT`），这里留一点余量，超过就拒绝。
const STAGED_LIMIT: u64 = 9 * 1024 * 1024;

/// 扫描一次导出根，每个文件恰好给出一条事实；任何一个文件归不了类，整批拒绝。
pub fn read_trusted_export(
    export_root: &Path,
    vault: VaultUuid,
) -> Result<Vec<TrustedExportFact>, BrokerError> {
    let mut files = BTreeMap::new();
    walk(export_root, export_root, vault, &mut files, 0)?;
    let approvals = attempt_approvals(export_root, &files)?;
    let owners = history_owners(export_root, &files)?;
    let staged = staged_approvals(export_root, vault, &files)?;
    files
        .iter()
        .map(|(path, revision)| {
            let (class, provenance) = match staged.get(path) {
                Some(approval) => (TrustedExportClass::SourceMaterial, Some(approval.clone())),
                None => classify(path, &approvals, &owners)?,
            };
            Ok(TrustedExportFact {
                export_path: path.clone(),
                class,
                provenance,
                revision: *revision,
            })
        })
        .collect()
}

/// 一个导出文件的类别；来源资料附带它的原批准。
fn classify(
    path: &VaultPath,
    approvals: &BTreeMap<VaultPath, ArtifactProvenance>,
    owners: &BTreeMap<VaultPath, VaultPath>,
) -> Result<(TrustedExportClass, Option<ArtifactProvenance>), BrokerError> {
    if let Some(logical) = attempt_logical(path) {
        let approval = approvals
            .get(logical)
            .ok_or_else(|| rejected(path, "尝试目录里的文件没有被本目录的清单列出"))?;
        return Ok((TrustedExportClass::SourceMaterial, Some(approval.clone())));
    }
    if is_history_body(path) {
        let document = owners
            .get(path)
            .ok_or_else(|| rejected(path, "历史正文没有目录认领"))?;
        return match approvals.get(document) {
            // 来源资料的旧版本仍是来源资料：跟着原批准的策略和期限走，到期一样只留非正文记录。
            // 归进「完整范围 L0」的话它就成了必含类别，期限对它不起作用。
            Some(approval) if approval.labels().backup_scope() == BackupScope::Full => {
                Ok((TrustedExportClass::SourceMaterial, Some(approval.clone())))
            }
            Some(_) => Err(rejected(path, "不是完整范围的来源资料不该有共享历史")),
            None if document.as_str().starts_with(ATTEMPTS) => {
                Err(rejected(path, "尝试正文的历史版本找不到原清单"))
            }
            None => Ok((TrustedExportClass::FullScopeL0, None)),
        };
    }
    let class = owner_class(path).ok_or_else(|| rejected(path, "不认识的顶层目录"))?;
    Ok((class, None))
}

/// 写者数据根里的暂存正文各自带的原批准。读不出来（升级前的旧暂存、内容和文件名对不上）就整批拒绝。
fn staged_approvals(
    export_root: &Path,
    vault: VaultUuid,
    files: &BTreeMap<VaultPath, Revision>,
) -> Result<BTreeMap<VaultPath, ArtifactProvenance>, BrokerError> {
    let mut staged = BTreeMap::new();
    for path in files.keys() {
        let Some(inside) = path.as_str().strip_prefix(WRITER_DATA) else {
            continue;
        };
        let name = inside.rsplit('/').next().unwrap_or(inside);
        if !crate::artifact_stage::is_staged_name(name) {
            continue;
        }
        // Broker 只把暂存写在写者数据根这一层。名字像暂存、位置却不对的，不能漏到别的归类里。
        if name != inside {
            return Err(rejected(path, "暂存正文不在写者数据根这一层"));
        }
        let bytes = read_limited(export_root, path, STAGED_LIMIT)?;
        let approval = crate::artifact_stage::staged_provenance(vault, name, &bytes)
            .map_err(|_| rejected(path, "暂存正文没有原批准，或内容和文件名对不上"))?;
        staged.insert(path.clone(), approval);
    }
    Ok(staged)
}

/// 每个尝试目录的清单认领了哪些逻辑路径（清单自己也算），各自的原批准是什么。
fn attempt_approvals(
    export_root: &Path,
    files: &BTreeMap<VaultPath, Revision>,
) -> Result<BTreeMap<VaultPath, ArtifactProvenance>, BrokerError> {
    let mut approvals = BTreeMap::new();
    for path in files.keys() {
        let Some(directory) = attempt_manifest_directory(path) else {
            continue;
        };
        let bytes = read_limited(export_root, path, MANIFEST_LIMIT)?;
        let manifest: ManifestView =
            serde_json::from_slice(&bytes).map_err(|_| rejected(path, "清单读不出来"))?;
        let approval = match (manifest.format_version, manifest.provenance) {
            (2, Some(approval)) => approval,
            _ => return Err(old_manifest(path)),
        };
        let own = VaultPath::parse(format!("{directory}{MANIFEST_NAME}"))
            .map_err(|_| rejected(path, "清单路径不合法"))?;
        claim(&mut approvals, own, &approval, path)?;
        for artifact in manifest.artifacts {
            // 清单只能认领自己目录里的文件。不然一份清单就能把别处的必含内容说成会到期的来源资料。
            if !artifact.path.as_str().starts_with(directory) {
                return Err(rejected(path, "清单列了本目录以外的文件"));
            }
            // 清单列了、文件已经不在的不算错：没有东西要备份，但它的历史版本仍按这份批准归类。
            claim(&mut approvals, artifact.path, &approval, path)?;
        }
    }
    Ok(approvals)
}

fn claim(
    approvals: &mut BTreeMap<VaultPath, ArtifactProvenance>,
    logical: VaultPath,
    approval: &ArtifactProvenance,
    manifest: &VaultPath,
) -> Result<(), BrokerError> {
    if approvals.insert(logical, approval.clone()).is_some() {
        return Err(rejected(manifest, "同一个文件被清单认领了两次"));
    }
    Ok(())
}

/// 每份历史正文（导出路径）属于哪篇文档（逻辑路径）。
fn history_owners(
    export_root: &Path,
    files: &BTreeMap<VaultPath, Revision>,
) -> Result<BTreeMap<VaultPath, VaultPath>, BrokerError> {
    let mut owners = BTreeMap::new();
    for path in files.keys() {
        let Some(stem) = history_catalog_stem(path) else {
            continue;
        };
        let bytes = read_limited(export_root, path, CATALOG_LIMIT)?;
        let catalog: HistoryCatalog =
            serde_json::from_slice(&bytes).map_err(|_| rejected(path, "历史目录读不出来"))?;
        // 目录的文件名就是所属文档路径的摘要；对不上说明这份目录被挪给了别的文档。
        if !matches!(catalog.version, 1..=5) || sha256(catalog.path.as_str().as_bytes()) != stem {
            return Err(rejected(path, "历史目录的版本或文件名对不上"));
        }
        // 等待删除的旧正文可能还在盘上，仍算这篇文档的；文件已经删掉的那几条不会被用到。
        let revisions = catalog
            .entries
            .iter()
            .map(|item| item.entry.revision)
            .chain(catalog.garbage.iter().copied());
        for revision in revisions {
            let body = VaultPath::parse(format!("{HISTORY}{revision}"))
                .map_err(|_| rejected(path, "历史正文的名字不合法"))?;
            // 同一篇文档改回旧内容时，会有两条记录指向同一份正文（真实目录只要求记录编号不重复）；
            // 指向另一篇文档才是不一致。
            if owners
                .insert(body, catalog.path.clone())
                .is_some_and(|previous| previous != catalog.path)
            {
                return Err(rejected(path, "同一份历史正文被两篇文档认领"));
            }
        }
    }
    Ok(owners)
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

/// 尝试目录里的文件的逻辑路径（去掉导出里的 `canonical/` 前缀）；别处的文件返回 `None`。
fn attempt_logical(path: &VaultPath) -> Option<&str> {
    path.as_str()
        .strip_prefix(CANONICAL)
        .filter(|logical| logical.starts_with(ATTEMPTS))
}

/// 清单所在的尝试目录（逻辑路径，带结尾的斜杠）。清单只在「任务/尝试」这一层；
/// 更深处叫 `manifest.json` 的是产物，不是清单。
fn attempt_manifest_directory(path: &VaultPath) -> Option<&str> {
    let logical = attempt_logical(path)?;
    let directory = logical.strip_suffix(MANIFEST_NAME)?;
    let mut parts = directory.strip_prefix(ATTEMPTS)?.split('/');
    let exact = parts.next().is_some_and(|task| !task.is_empty())
        && parts.next().is_some_and(|attempt| !attempt.is_empty())
        && parts.next() == Some("")
        && parts.next().is_none();
    exact.then_some(directory)
}

/// 历史正文直接放在 `history/` 下，名字是它的 revision；`meta-v1/` 里的是元数据。
fn is_history_body(path: &VaultPath) -> bool {
    path.as_str()
        .strip_prefix(HISTORY)
        .is_some_and(|rest| rest.split('/').next() != Some(HISTORY_METADATA))
}

/// 每篇文档一份历史目录，文件名是逻辑路径的 SHA-256。
/// 同一个文件夹里的变更流分页（`feed-v1-N.json`）是另一种东西，按 owner 文件归类。
fn history_catalog_stem(path: &VaultPath) -> Option<&str> {
    path.as_str()
        .strip_prefix(HISTORY_CATALOGS)?
        .strip_suffix(".json")
        .filter(|stem| {
            stem.len() == 64
                && stem
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
}

fn walk(
    export_root: &Path,
    directory: &Path,
    vault: VaultUuid,
    files: &mut BTreeMap<VaultPath, Revision>,
    depth: u32,
) -> Result<(), BrokerError> {
    if depth > MAX_DEPTH {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let entries = fs::read_dir(directory).map_err(|error| BrokerError::io("read export", error))?;
    for entry in entries {
        let absolute = entry
            .map_err(|error| BrokerError::io("read export entry", error))?
            .path();
        let metadata = fs::symlink_metadata(&absolute)
            .map_err(|error| BrokerError::io("stat export", error))?;
        if metadata.is_dir() {
            walk(export_root, &absolute, vault, files, depth + 1)?;
            continue;
        }
        let path = absolute
            .strip_prefix(export_root)
            .ok()
            .and_then(Path::to_str)
            .and_then(|relative| VaultPath::parse(relative).ok())
            .ok_or_else(|| BrokerError::InvalidArtifact("导出里有路径不合法的文件".into()))?;
        // 符号链接、设备、套接字这些都不是 owner 导出该有的东西。
        if !metadata.is_file() {
            return Err(rejected(&path, "不是普通文件"));
        }
        // 崩溃时写到一半的临时文件：里面可能是来源资料的半截或全文，没法归类，只能拒绝。
        if absolute
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(crate::fs::is_temporary_name)
        {
            return Err(rejected(&path, "写到一半留下的临时文件"));
        }
        if files.len() >= MAX_FILES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let revision = hash_file(&absolute, vault, &path)?;
        files.insert(path, revision);
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

fn read_limited(export_root: &Path, path: &VaultPath, limit: u64) -> Result<Vec<u8>, BrokerError> {
    let absolute = export_root.join(path.as_str());
    let metadata = fs::symlink_metadata(&absolute)
        .map_err(|error| BrokerError::io("stat export file", error))?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(rejected(path, "不是普通文件或超过大小上限"));
    }
    fs::read(&absolute).map_err(|error| BrokerError::io("read export file", error))
}

fn rejected(path: &VaultPath, why: &str) -> BrokerError {
    BrokerError::InvalidArtifact(format!("导出文件没有可接受的来源登记：{why}（{path}）"))
}

fn old_manifest(path: &VaultPath) -> BrokerError {
    BrokerError::InvalidArtifact(format!("旧清单缺少来源元数据，不能升为完整范围（{path}）"))
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

/// 历史目录里本模块用得到的字段；其余字段的校验是历史模块自己的事。
#[derive(Deserialize)]
struct HistoryCatalog {
    version: u32,
    path: VaultPath,
    entries: Vec<HistoryItem>,
    #[serde(default)]
    garbage: Vec<Revision>,
}

#[derive(Deserialize)]
struct HistoryItem {
    entry: HistoryEntry,
}

#[derive(Deserialize)]
struct HistoryEntry {
    revision: Revision,
}
