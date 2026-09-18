//! 白名单一致性诊断：只读地检查已配置的 SyncScope／file_operations／human_content
//! 生成区之间有没有互相矛盾的登记。
//!
//! 诊断**不改配置**，也不代替 host-config 自己的校验；它回答的是
//! "这几张表放在一起讲不讲得通"，结论只进健康页。
//!
//! 输入是一份纯快照 DTO：`observability` 不依赖 `host-config`，
//! 由装配层把已加载的配置摊平成这份快照再传进来。

use studiolo_core::VaultPath;

/// 白名单快照。每一栏都是已经由 host-config 校验过的登记路径。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WhitelistSnapshot {
    /// SyncScope 登记的整文件。
    pub sync_whole_files: Vec<VaultPath>,
    /// SyncScope 登记的整目录前缀。
    pub sync_whole_folders: Vec<VaultPath>,
    /// SyncScope 继承的保护前缀（人写正文 protected）。
    pub sync_protected: Vec<VaultPath>,
    /// 生成区：由机器产出、人不得双向搬运的文档。
    pub generated_regions: Vec<VaultPath>,
    /// file_operations 允许人侧删除/改名的整文件范围。
    pub file_operation_writable: Vec<VaultPath>,
    /// human_content 登记的人写目录。
    pub human_content_folders: Vec<VaultPath>,
    /// human_content 登记的保护前缀。
    pub human_content_protected: Vec<VaultPath>,
}

impl WhitelistSnapshot {
    fn total(&self) -> usize {
        self.sync_whole_files.len()
            + self.sync_whole_folders.len()
            + self.sync_protected.len()
            + self.generated_regions.len()
            + self.file_operation_writable.len()
            + self.human_content_folders.len()
            + self.human_content_protected.len()
    }
}

/// 诊断本身失败的固定分类；失败即健康页"未知"，不得当成"没有冲突"。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DiagnosticsError {
    /// 登记总数超过诊断预算；宁可判未知也不做无界比较。
    #[error("白名单登记总数超过诊断预算")]
    TooManyEntries,
}

/// 单条冲突的固定分类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WhitelistFindingKind {
    /// 同一路径既登记为生成区，又落在人侧可写范围内。
    GeneratedRegionIsWritable,
    /// 同一路径既登记为生成区，又落在 Sync 搬运范围内。
    GeneratedRegionInsideSyncScope,
    /// Sync 搬运范围覆盖了保护前缀。
    SyncScopeCoversProtected,
    /// 人写目录整体落在保护前缀之内，等于永远批不出来。
    HumanFolderInsideProtected,
    /// 人侧可写范围覆盖了保护前缀。
    WritableCoversProtected,
    /// 同一路径在同一张表里重复登记。
    DuplicateRegistration,
    /// 整文件登记同时被自己所在的整目录规则覆盖，两条规则会互相遮蔽。
    WholeFileShadowedByFolder,
}

impl WhitelistFindingKind {
    /// 稳定机器码。
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Self::GeneratedRegionIsWritable => "generated-region-is-writable",
            Self::GeneratedRegionInsideSyncScope => "generated-region-inside-sync-scope",
            Self::SyncScopeCoversProtected => "sync-scope-covers-protected",
            Self::HumanFolderInsideProtected => "human-folder-inside-protected",
            Self::WritableCoversProtected => "writable-covers-protected",
            Self::DuplicateRegistration => "duplicate-registration",
            Self::WholeFileShadowedByFolder => "whole-file-shadowed-by-folder",
        }
    }

    /// 中文说明；不含路径。
    #[must_use]
    pub fn description(self) -> &'static str {
        match self {
            Self::GeneratedRegionIsWritable => "生成区同时落在人侧可写范围内",
            Self::GeneratedRegionInsideSyncScope => "生成区同时落在双向搬运范围内",
            Self::SyncScopeCoversProtected => "搬运范围覆盖了保护前缀",
            Self::HumanFolderInsideProtected => "人写目录整体落在保护前缀之内",
            Self::WritableCoversProtected => "人侧可写范围覆盖了保护前缀",
            Self::DuplicateRegistration => "同一张登记表内重复登记",
            Self::WholeFileShadowedByFolder => "整文件登记被同范围的整目录规则遮蔽",
        }
    }
}

/// 一条冲突。`path` 只在本地结论里出现，公开摘要不带它。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhitelistFinding {
    /// 冲突分类。
    pub kind: WhitelistFindingKind,
    /// 触发冲突的登记路径。
    pub path: VaultPath,
}

/// 诊断结论。只读，绝不代表已经或将要修改任何配置。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhitelistDiagnosis {
    examined: usize,
    findings: Vec<WhitelistFinding>,
}

impl WhitelistDiagnosis {
    /// 本次检查覆盖的登记条目数。
    #[must_use]
    pub fn examined(&self) -> usize {
        self.examined
    }

    /// 发现的冲突，按分类与路径稳定排序。
    #[must_use]
    pub fn findings(&self) -> &[WhitelistFinding] {
        &self.findings
    }

    /// 是否一致。
    #[must_use]
    pub fn consistent(&self) -> bool {
        self.findings.is_empty()
    }
}

const ENTRY_BUDGET: usize = 4096;

/// 只读一致性检查；不修改快照，也不产生任何修复动作。
pub fn diagnose(snapshot: &WhitelistSnapshot) -> Result<WhitelistDiagnosis, DiagnosticsError> {
    let examined = snapshot.total();
    if examined > ENTRY_BUDGET {
        return Err(DiagnosticsError::TooManyEntries);
    }
    let mut findings = Vec::new();

    for (table, kind) in [
        (
            &snapshot.sync_whole_files,
            WhitelistFindingKind::DuplicateRegistration,
        ),
        (
            &snapshot.sync_whole_folders,
            WhitelistFindingKind::DuplicateRegistration,
        ),
        (
            &snapshot.sync_protected,
            WhitelistFindingKind::DuplicateRegistration,
        ),
        (
            &snapshot.generated_regions,
            WhitelistFindingKind::DuplicateRegistration,
        ),
        (
            &snapshot.file_operation_writable,
            WhitelistFindingKind::DuplicateRegistration,
        ),
        (
            &snapshot.human_content_folders,
            WhitelistFindingKind::DuplicateRegistration,
        ),
        (
            &snapshot.human_content_protected,
            WhitelistFindingKind::DuplicateRegistration,
        ),
    ] {
        let mut seen = std::collections::BTreeSet::new();
        for path in table {
            if !seen.insert(path.collision_key()) {
                findings.push(WhitelistFinding {
                    kind,
                    path: path.clone(),
                });
            }
        }
    }

    for generated in &snapshot.generated_regions {
        if snapshot
            .file_operation_writable
            .iter()
            .any(|prefix| covers(prefix, generated))
        {
            findings.push(WhitelistFinding {
                kind: WhitelistFindingKind::GeneratedRegionIsWritable,
                path: generated.clone(),
            });
        }
        if snapshot
            .sync_whole_files
            .iter()
            .chain(&snapshot.sync_whole_folders)
            .any(|prefix| covers(prefix, generated))
        {
            findings.push(WhitelistFinding {
                kind: WhitelistFindingKind::GeneratedRegionInsideSyncScope,
                path: generated.clone(),
            });
        }
    }

    for protected in snapshot
        .sync_protected
        .iter()
        .chain(&snapshot.human_content_protected)
    {
        if snapshot
            .sync_whole_files
            .iter()
            .chain(&snapshot.sync_whole_folders)
            .any(|prefix| covers(prefix, protected))
        {
            findings.push(WhitelistFinding {
                kind: WhitelistFindingKind::SyncScopeCoversProtected,
                path: protected.clone(),
            });
        }
        if snapshot
            .file_operation_writable
            .iter()
            .any(|prefix| covers(prefix, protected))
        {
            findings.push(WhitelistFinding {
                kind: WhitelistFindingKind::WritableCoversProtected,
                path: protected.clone(),
            });
        }
    }

    for folder in &snapshot.human_content_folders {
        if snapshot
            .human_content_protected
            .iter()
            .any(|prefix| covers(prefix, folder))
        {
            findings.push(WhitelistFinding {
                kind: WhitelistFindingKind::HumanFolderInsideProtected,
                path: folder.clone(),
            });
        }
    }

    for file in &snapshot.sync_whole_files {
        if snapshot
            .sync_whole_folders
            .iter()
            .any(|folder| covers(folder, file) && folder != file)
        {
            findings.push(WhitelistFinding {
                kind: WhitelistFindingKind::WholeFileShadowedByFolder,
                path: file.clone(),
            });
        }
    }

    findings.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.path.as_str().cmp(right.path.as_str()))
    });
    findings.dedup();
    Ok(WhitelistDiagnosis { examined, findings })
}

/// `prefix` 是否覆盖 `path`；相等也算覆盖。只做保守的段边界比较。
fn covers(prefix: &VaultPath, path: &VaultPath) -> bool {
    let prefix = prefix.as_str();
    let path = path.as_str();
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && path.as_bytes().get(prefix.len()) == Some(&b'/'))
}
