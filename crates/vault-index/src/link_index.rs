//! wikilink 解析索引：把「路径键」与「名称键」预聚合成候选集合，使单条链接解析只访问
//! 候选，不再逐条遍历全库。
//!
//! 索引是纯派生物：内容态仍然只在文件里，索引丢失后可由当前文档集合完全重建
//! （`LinkIndex::rebuilt`），重建结果与逐篇增量维护（`LinkIndex::apply`）必须一致。

use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;

use crate::{FrontmatterValue, IndexedDocument};

/// 一篇笔记参与链接解析的全部比较键。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocumentKeys {
    path_key: String,
    names: BTreeSet<String>,
}

/// 本次增量维护实际触碰的篇数；用于证明未变文档不被重复索引。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkIndexUpdate {
    pub(crate) inserted: usize,
    pub(crate) removed: usize,
    pub(crate) unchanged: usize,
}

/// 按比较键聚合的候选集合；每个候选列表按 `VaultPath` 稳定排序且不含重复。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkIndex {
    by_path_key: BTreeMap<String, Vec<VaultPath>>,
    by_name: BTreeMap<String, Vec<VaultPath>>,
    keys: BTreeMap<VaultPath, DocumentKeys>,
}

impl LinkIndex {
    /// 从当前文档集合完全重建索引；这是「丢了索引也能恢复」的那条路径。
    pub(crate) fn rebuilt(documents: &BTreeMap<VaultPath, IndexedDocument>) -> Self {
        let mut index = Self::default();
        for document in documents.values() {
            index.insert(&document.path, document_keys(document));
        }
        index
    }

    /// 索引是否为空；空索引走完全重建，其余代次走增量维护。
    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// 增量维护：删除本轮消失的文档，只重索引键发生变化的文档，其余原样保留。
    pub(crate) fn apply(
        &mut self,
        documents: &BTreeMap<VaultPath, IndexedDocument>,
    ) -> LinkIndexUpdate {
        let mut update = LinkIndexUpdate::default();
        let stale: Vec<VaultPath> = self
            .keys
            .keys()
            .filter(|path| !documents.contains_key(*path))
            .cloned()
            .collect();
        for path in stale {
            self.remove(&path);
            update.removed += 1;
        }
        for (path, document) in documents {
            let keys = document_keys(document);
            if self.keys.get(path) == Some(&keys) {
                update.unchanged += 1;
                continue;
            }
            self.remove(path);
            self.insert(path, keys);
            update.inserted += 1;
        }
        update
    }

    /// 候选路径：整条逻辑路径（含 `.md`）大小写折叠后精确相等的文档。
    pub(crate) fn paths_for(&self, key: &str) -> &[VaultPath] {
        self.by_path_key.get(key).map_or(&[], Vec::as_slice)
    }

    /// 候选路径：文件名、展示标题或 frontmatter alias 命中该名称键的文档。
    pub(crate) fn names_for(&self, key: &str) -> &[VaultPath] {
        self.by_name.get(key).map_or(&[], Vec::as_slice)
    }

    fn insert(&mut self, path: &VaultPath, keys: DocumentKeys) {
        push_sorted(
            self.by_path_key.entry(keys.path_key.clone()).or_default(),
            path,
        );
        for name in &keys.names {
            push_sorted(self.by_name.entry(name.clone()).or_default(), path);
        }
        self.keys.insert(path.clone(), keys);
    }

    fn remove(&mut self, path: &VaultPath) {
        let Some(keys) = self.keys.remove(path) else {
            return;
        };
        remove_sorted(&mut self.by_path_key, &keys.path_key, path);
        for name in &keys.names {
            remove_sorted(&mut self.by_name, name, path);
        }
    }
}

fn push_sorted(bucket: &mut Vec<VaultPath>, path: &VaultPath) {
    if let Err(position) = bucket.binary_search(path) {
        bucket.insert(position, path.clone());
    }
}

fn remove_sorted(buckets: &mut BTreeMap<String, Vec<VaultPath>>, key: &str, path: &VaultPath) {
    let Some(bucket) = buckets.get_mut(key) else {
        return;
    };
    if let Ok(position) = bucket.binary_search(path) {
        bucket.remove(position);
    }
    if bucket.is_empty() {
        buckets.remove(key);
    }
}

fn document_keys(document: &IndexedDocument) -> DocumentKeys {
    DocumentKeys {
        path_key: comparison_key(document.path.as_str()),
        names: document_names(document),
    }
}

/// 名称候选：文件名去 `.md`、展示标题与 frontmatter `aliases`，全部折叠成比较键。
fn document_names(document: &IndexedDocument) -> BTreeSet<String> {
    let filename = document
        .path
        .as_str()
        .rsplit('/')
        .next()
        .unwrap_or(document.path.as_str());
    let mut names = BTreeSet::from([
        comparison_key(stem(filename)),
        comparison_key(&document.title.text),
    ]);
    if let Some(value) = document.frontmatter.get("aliases") {
        match value {
            FrontmatterValue::Text(alias) => {
                names.insert(comparison_key(alias));
            }
            FrontmatterValue::List(aliases) => {
                for alias in aliases {
                    if let FrontmatterValue::Text(alias) = alias {
                        names.insert(comparison_key(alias));
                    }
                }
            }
            _ => {}
        }
    }
    names
}

/// 去掉大小写不敏感的 `.md` 扩展名；其余点号一律保留。
pub(crate) fn stem(filename: &str) -> &str {
    filename
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("md"))
        .map_or(filename, |(stem, _)| stem)
}

/// 唯一的键折叠点：去首尾空白并做 Unicode 小写。路径键、文件名键、标题键、别名键与链接
/// 文本键全部经过这里，所以折叠规则只有一处可改，不会出现两侧规则不一致。
///
/// `VaultPath` 已由 `studiolo_core` 强制为 NFC，所以路径、文件名与标题侧的键天然是 NFC；
/// 正文里手写的链接文本仍可能是分解拼写，那种链接目前会悬空。补法是给本 crate 接上
/// workspace 已声明的 `unicode-normalization`（root 接线范围，见 T-054 REPORT），再把本
/// 函数改成先 `.nfc().collect::<String>()` 再 trim + 小写；改动只落在这一个函数上，
/// 回归由 `tests/link_index_rebuild_contract.rs` 的 NFC 用例守住。
pub(crate) fn comparison_key(value: &str) -> String {
    // 路径侧天然是 NFC（`studiolo_core::VaultPath` 直接拒收分解拼写），而正文里手写的链接
    // 文本没有这层保证。先归一到 NFC 再折叠大小写，两侧才落在同一个键上；否则同一个名字的
    // 分解拼写会静默悬空。与 `studiolo_core` 的归一化口径保持一致。
    use unicode_normalization::UnicodeNormalization as _;
    value.trim().nfc().collect::<String>().to_lowercase()
}

#[cfg(test)]
#[path = "link_index_tests.rs"]
mod tests;
