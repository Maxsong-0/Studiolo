//! 私有配置中的人写正文登记；目录/文件内容不能自行取得写权限。

use crate::ConfigError;
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use studiolo_core::{VaultPath, VaultUuid};

/// 与特定 Vault 绑定的可信正文范围；无公开构造或反序列化入口。
#[derive(Clone)]
pub struct HumanContentScope(Document);

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    vault_uuid: VaultUuid,
    folders: Vec<VaultPath>,
    protected: Vec<VaultPath>,
    // 显式登记混合文档；每一对字符串是占据整行的不可变区首尾哨兵。
    documents: BTreeMap<VaultPath, Vec<[String; 2]>>,
}

pub(crate) fn decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<Option<HumanContentScope>, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Option<HumanContentScope>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("人写正文范围对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            Document::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                .map(|doc| Some(HumanContentScope(doc)))
        }
    }
    input.deserialize_map(Object)
}

impl HumanContentScope {
    /// Sync 只能继承禁止项，不能借人写目录批准整页复制或修改本范围。
    pub(crate) fn sync_exclusions(
        &self,
        vault: VaultUuid,
    ) -> Result<(Vec<VaultPath>, Vec<VaultPath>), ConfigError> {
        if self.0.vault_uuid != vault {
            return Err(ConfigError);
        }
        Ok((
            self.0.protected.clone(),
            self.0
                .documents
                .iter()
                .filter(|(_, regions)| !regions.is_empty())
                .map(|(path, _)| path.clone())
                .collect(),
        ))
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        let doc = &self.0;
        if doc.version != 1
            || doc.folders.len() > 64
            || doc.protected.len() > 128
            || doc.documents.len() > 64
            || (doc.folders.is_empty() && doc.documents.is_empty())
        {
            return Err(ConfigError);
        }
        for path in doc.folders.iter().chain(doc.documents.keys()) {
            if hard_protected(path) {
                return Err(ConfigError);
            }
        }
        for regions in doc.documents.values() {
            if regions.len() > 16 {
                return Err(ConfigError);
            }
            let mut seen = std::collections::BTreeSet::new();
            for pair in regions {
                for marker in pair {
                    if marker.is_empty()
                        || marker.len() > 160
                        || marker.chars().any(char::is_control)
                        || !marker.starts_with("<!-- ")
                        || !marker.ends_with(" -->")
                        || !seen.insert(marker)
                    {
                        return Err(ConfigError);
                    }
                }
            }
        }
        Ok(())
    }

    /// 检查可信登记及固定保护边界；不根据扩展名/frontmatter自行授权。
    #[must_use]
    pub fn allows(&self, vault: VaultUuid, path: &VaultPath) -> bool {
        let doc = &self.0;
        if vault != doc.vault_uuid
            || hard_protected(path)
            || doc.protected.iter().any(|prefix| within(path, prefix))
        {
            return false;
        }
        if doc.documents.contains_key(path) {
            return true;
        }
        if matches!(
            path.as_str()
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_lowercase()
                .as_str(),
            "index.md" | "tasks.md" | "dashboard.md"
        ) {
            return false;
        }
        doc.folders.iter().any(|prefix| within(path, prefix))
    }

    /// 整文件操作不能使用混合文档的部分编辑权限；保留名、raw及生成区始终拒绝。
    pub(crate) fn allows_whole_file(&self, vault: VaultUuid, path: &VaultPath) -> bool {
        if !self.allows(vault, path) || crate::sync_scope::hard_excluded_whole_file(path) {
            return false;
        }
        let fold = |path: &VaultPath| {
            path.as_str()
                .chars()
                .flat_map(char::to_uppercase)
                .flat_map(char::to_lowercase)
                .collect::<String>()
        };
        let candidate = fold(path);
        !self
            .0
            .protected
            .iter()
            .chain(
                self.0
                    .documents
                    .iter()
                    .filter(|(_, regions)| !regions.is_empty())
                    .map(|(path, _)| path),
            )
            .any(|excluded| {
                let excluded = fold(excluded);
                candidate == excluded
                    || candidate
                        .strip_prefix(&excluded)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
    }

    // 新整文件配对把所有人写登记加入同一索引；本方法不扩大旧编辑许可。
    pub(crate) fn rename_registered_paths(&self) -> impl Iterator<Item = &VaultPath> {
        self.0
            .folders
            .iter()
            .chain(&self.0.protected)
            .chain(self.0.documents.keys())
    }

    /// 检查当前和拟写入文本的所有受保护区逐字节相同，拒绝缺失、重复和交叠哨兵。
    #[must_use]
    pub fn preserves_generated(&self, path: &VaultPath, before: &str, after: &str) -> bool {
        let Some(regions) = self.0.documents.get(path) else {
            return true;
        };
        let Some(left) = spans(before, regions) else {
            return false;
        };
        let Some(right) = spans(after, regions) else {
            return false;
        };
        left.iter()
            .zip(right.iter())
            .all(|(a, b)| before[a.clone()] == after[b.clone()])
    }
}

fn within(path: &VaultPath, prefix: &VaultPath) -> bool {
    path == prefix
        || path
            .as_str()
            .strip_prefix(prefix.as_str())
            .is_some_and(|s| s.starts_with('/'))
}

pub(crate) fn hard_protected(path: &VaultPath) -> bool {
    // 规则目录对人侧保存仍然保护：改规则要走策略平面与 T-160 的规则管理界面，
    // 不能从网页编辑器直接写；但它不再是同步范围的硬排除（用户 2026-09-23 拍板纳入）。
    reserved_for_agents(path) || rules_directory(path)
}

/// 人侧与同步范围共用的硬保留：core 保护路径、AI 专属目录与保留文档名。
///
/// 这份清单是唯一母本；`sync_scope::paths::hard_excluded` 直接复用，不再各抄一份。
pub(crate) fn reserved_for_agents(path: &VaultPath) -> bool {
    if path.is_protected() {
        return true;
    }
    let lower = path.as_str().to_lowercase();
    let parts: Vec<_> = lower.split('/').collect();
    parts.iter().any(|part| {
        matches!(
            *part,
            "skills"
                | "policies"
                | "capabilities"
                | "ops"
                | "sources"
                | "workbench_data"
                | "workbench_ephemeral"
        )
    }) || matches!(
        parts.last().copied(),
        Some("claude.md" | "agents.md" | "skill.md")
    )
}

/// `_规则/` 目录：只对人侧保存保护，同步范围可批准。
fn rules_directory(path: &VaultPath) -> bool {
    path.as_str().split('/').any(|part| part == "_规则")
}

fn spans(text: &str, pairs: &[[String; 2]]) -> Option<Vec<std::ops::Range<usize>>> {
    let mut out = Vec::with_capacity(pairs.len());
    for [start, end] in pairs {
        let find = |needle: &str| {
            let mut offset = 0;
            let mut found = None;
            for line in text.split_inclusive('\n') {
                if line.trim_end_matches(['\r', '\n']) == needle {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(offset..offset + line.len());
                }
                offset += line.len();
            }
            found
        };
        let start = find(start)?;
        let end = find(end)?;
        if start.end > end.start {
            return None;
        }
        out.push(start.start..end.end);
    }
    let mut ordered = out.clone();
    ordered.sort_by_key(|range| range.start);
    if ordered.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return None;
    }
    Some(out)
}
