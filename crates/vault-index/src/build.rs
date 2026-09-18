//! 全局链接仅针对本轮输入解析；旧缓存不能保留跨文档目标。
//!
//! 解析前先把 [`LinkIndex`] 增量推进到本轮文档集合：未变的文档不重新索引，消失的文档
//! 立即退出候选集合，链接解析随后只查候选，不再逐条遍历全库。

use crate::{
    InMemoryVaultIndex, IndexDiagnostic, IndexGeneration, IndexedDocument, link_index::LinkIndex,
    links,
};
use std::collections::BTreeMap;
use studiolo_core::VaultPath;

impl InMemoryVaultIndex {
    pub(crate) fn publish(
        &mut self,
        mut replacement: BTreeMap<VaultPath, IndexedDocument>,
    ) -> IndexGeneration {
        // 冷启动（索引为空）走完全重建，后续代次只推进差异；两条路径的结果必须一致，
        // 由 `link_index_tests` 的等价性回归守住。
        if self.link_index.is_empty() {
            self.link_index = LinkIndex::rebuilt(&replacement);
        } else {
            self.link_index.apply(&replacement);
        }

        let paths: Vec<_> = replacement.keys().cloned().collect();
        for path in &paths {
            let Some(document) = replacement.get(path) else {
                continue;
            };
            let targets: Vec<_> = document
                .outgoing_links
                .iter()
                .map(|link| link.target.clone())
                .collect();
            let resolutions: Vec<_> = targets
                .iter()
                .map(|target| links::resolve(&self.link_index, path, target))
                .collect();
            if let Some(document) = replacement.get_mut(path) {
                for (link, resolution) in document.outgoing_links.iter_mut().zip(resolutions) {
                    link.resolution = resolution;
                }
            }
        }

        let (backlinks, link_health) = links::derive_links(&replacement);

        self.generation.generation = self.generation.generation.saturating_add(1);
        self.generation.documents = replacement.len();
        self.generation.links = replacement
            .values()
            .map(|document| document.outgoing_links.len())
            .sum();
        self.diagnostics = replacement
            .values()
            .filter_map(|document| {
                document.unavailable.map(|reason| IndexDiagnostic {
                    path: document.path.clone(),
                    reason,
                })
            })
            .collect();
        self.documents = replacement;
        self.backlinks = backlinks;
        self.link_health = link_health;
        self.generation
    }
}
