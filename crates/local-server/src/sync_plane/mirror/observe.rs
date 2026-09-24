//! 镜像视图的完整观察：按批准的目录锚点递归、精确文件逐个核对、`required` 缺失显式 `Missing`。
//!
//! 只编排，不打开路径：目录枚举、节点分类与流式摘要都借 `stage.rs`／`content.rs`。
//! 硬排除与 `directory_for_observation` 同一口径——母本是 host-config 的
//! `human_content::reserved_for_agents` 与 `sync_scope::paths::hard_excluded`（vault-broker 的
//! `sync_observation::paths::hard` 是另一份复制）：隐藏名、AI 专属目录、保留文档名与
//! `excluded_prefixes` 一律不下探、不计入；契约测试用 `SyncScope` 反向核对这份复制。

use super::{MirrorView, PlaneError, content, descend, open_view, split, stage};
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;
use studiolo_host_config::{SyncObservationSelection, SyncTraversalRule};
use studiolo_observability::quota::SyncUsage;
use studiolo_sync_bridge::{Binding, FileVersion, Inventory, Presence, Side};

/// 观察里最多接受的文件数；与规划器的整批路径上限一致。
const FILE_LIMIT: usize = 4096;
/// 路径分量上限；与范围索引一致。
const DEPTH_LIMIT: usize = 64;

/// 一次镜像完整观察的结果：清单加字节合计（健康页 `sync-quota` 用）。
pub struct MirrorObservation {
    /// 完整清单；缺失显式 `Missing`。
    pub inventory: Inventory,
    /// 范围内对象数与字节数。
    pub usage: SyncUsage,
}

/// 在静止窗口内完整观察镜像视图：按 `selection` 遍历，逐文件流式 sha256，
/// 链接／特殊文件／越权名字任一出现即 `complete=false`（不 panic、不中止）。
///
/// # Errors
/// 视图身份漂移返回 `Observation`；其它局部失败只让 `complete=false`。
pub fn observe(
    view: &MirrorView,
    selection: &SyncObservationSelection,
    binding: &Binding,
    sequence: u64,
    required: &[VaultPath],
) -> Result<MirrorObservation, PlaneError> {
    let mut walk = Walk {
        excluded: selection.excluded_prefixes(),
        entries: BTreeMap::new(),
        complete: true,
        usage: SyncUsage {
            objects: 0,
            bytes: 0,
        },
    };
    for rule in selection.folders() {
        let parts: Vec<&str> = rule.path().as_str().split('/').collect();
        match descend(open_view(view)?, &parts, false) {
            Ok(Some(directory)) => {
                walk.directory(&directory, rule.path().as_str(), rule.policy().max_bytes());
            }
            Ok(None) => {}
            Err(_) => walk.complete = false,
        }
    }
    for rule in selection.files() {
        walk.exact(view, rule)?;
    }
    for path in required {
        if walk.entries.contains_key(path) {
            continue;
        }
        if in_scope(selection, path) {
            walk.entries.insert(path.clone(), Presence::Missing);
        } else {
            walk.complete = false;
        }
    }
    Ok(MirrorObservation {
        inventory: Inventory {
            binding: binding.clone(),
            side: Side::Mirror,
            sequence,
            complete: walk.complete,
            entries: walk.entries,
        },
        usage: walk.usage,
    })
}

struct Walk<'a> {
    excluded: &'a [VaultPath],
    entries: BTreeMap<VaultPath, Presence>,
    complete: bool,
    usage: SyncUsage,
}

impl Walk<'_> {
    /// 递归一个目录：隐藏名、硬排除与排除前缀不下探；同层大小写碰撞、不可表示的名字、
    /// 链接与特殊节点都让观察不完整。
    fn directory(&mut self, directory: &OwnedFd, prefix: &str, max_bytes: u64) {
        if prefix.split('/').count() >= DEPTH_LIMIT {
            self.complete = false;
            return;
        }
        let Ok(names) = stage::entry_names(directory.as_fd()) else {
            self.complete = false;
            return;
        };
        let mut keys = BTreeSet::new();
        for raw in names {
            let Some(name) = raw.to_str() else {
                self.complete = false;
                continue;
            };
            let Ok(path) = VaultPath::parse(format!("{prefix}/{name}")) else {
                self.complete = false;
                continue;
            };
            if !keys.insert(fold(name)) {
                self.complete = false;
                continue;
            }
            if hard(&path) || self.excluded.iter().any(|prefix| within(&path, prefix)) {
                continue;
            }
            match stage::open_node_at(directory.as_fd(), name) {
                Ok(stage::Opened::Directory(child)) => {
                    self.directory(&child, path.as_str(), max_bytes);
                }
                Ok(stage::Opened::Regular {
                    file,
                    bytes,
                    unique: true,
                }) => self.file(path, file.as_fd(), bytes, max_bytes),
                _ => self.complete = false,
            }
        }
    }

    /// 精确文件规则：缺失显式 `Missing`，父链断了也算缺失；链接或非文件让观察不完整。
    fn exact(&mut self, view: &MirrorView, rule: &SyncTraversalRule) -> Result<(), PlaneError> {
        let path = rule.path();
        let (parents, name) = split(path);
        match descend(open_view(view)?, &parents, false) {
            Ok(Some(parent)) => match stage::open_node_at(parent.as_fd(), name) {
                Ok(stage::Opened::Missing) => {
                    self.entries.insert(path.clone(), Presence::Missing);
                }
                Ok(stage::Opened::Regular {
                    file,
                    bytes,
                    unique: true,
                }) => self.file(path.clone(), file.as_fd(), bytes, rule.policy().max_bytes()),
                _ => self.complete = false,
            },
            Ok(None) => {
                self.entries.insert(path.clone(), Presence::Missing);
            }
            Err(_) => self.complete = false,
        }
        Ok(())
    }

    /// 流式摘要一个普通文件；超限、超出文件数或读取期间变化都让观察不完整。
    fn file(&mut self, path: VaultPath, file: BorrowedFd<'_>, bytes: u64, max_bytes: u64) {
        if bytes > max_bytes || self.entries.len() >= FILE_LIMIT {
            self.complete = false;
            return;
        }
        match content::stream(file, max_bytes, None) {
            Ok((digest, read)) if read == bytes => {
                self.usage.objects += 1;
                self.usage.bytes = self.usage.bytes.saturating_add(bytes);
                let version = FileVersion {
                    revision: digest,
                    sha256: digest,
                    bytes,
                };
                self.entries.insert(path, Presence::Present(version));
            }
            _ => self.complete = false,
        }
    }
}

/// `required` 里的路径是否落在批准范围内（目录锚点之下或精确文件本身，且未被排除）。
fn in_scope(selection: &SyncObservationSelection, path: &VaultPath) -> bool {
    if hard(path)
        || selection
            .excluded_prefixes()
            .iter()
            .any(|prefix| within(path, prefix))
    {
        return false;
    }
    selection.files().iter().any(|rule| rule.path() == path)
        || selection
            .folders()
            .iter()
            .any(|rule| rule.path() != path && within(path, rule.path()))
}

/// 与 host-config `sync_scope::paths::fold` 同一保守折叠：大写再小写合并 ß／SS 之类。
fn fold(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect()
}

fn within(path: &VaultPath, prefix: &VaultPath) -> bool {
    let path = fold(path.as_str());
    let prefix = fold(prefix.as_str());
    path == prefix
        || path
            .strip_prefix(&prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// 双向固定硬排除（与 host-config `sync_scope::paths::hard_excluded` 逐项相同）：
/// 隐藏名、`raw`／`ops`／工作台与 AI 专属目录、六个保留文档名。
fn hard(path: &VaultPath) -> bool {
    let value = fold(path.as_str());
    let parts: Vec<&str> = value.split('/').collect();
    parts.iter().any(|part| {
        part.starts_with('.')
            || matches!(
                *part,
                "raw"
                    | "ops"
                    | "workbench_data"
                    | "workbench_ephemeral"
                    | "skills"
                    | "policies"
                    | "capabilities"
                    | "sources"
            )
    }) || matches!(
        parts.last().copied(),
        Some("tasks.md" | "index.md" | "dashboard.md" | "claude.md" | "agents.md" | "skill.md")
    )
}
