//! 全局链接仅针对本轮输入解析；旧缓存不能保留跨文档目标。

use crate::{InMemoryVaultIndex, IndexGeneration, IndexedDocument, links};
use std::collections::BTreeMap;
use studiolo_core::VaultPath;

impl InMemoryVaultIndex {
    pub(crate) fn publish(
        &mut self,
        mut replacement: BTreeMap<VaultPath, IndexedDocument>,
    ) -> IndexGeneration {
        let paths: Vec<_> = replacement.keys().cloned().collect();
        for path in &paths {
            let targets: Vec<_> = replacement[path]
                .outgoing_links
                .iter()
                .map(|link| link.target.clone())
                .collect();
            let resolutions: Vec<_> = targets
                .iter()
                .map(|target| links::resolve(&replacement, path, target))
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
        self.documents = replacement;
        self.backlinks = backlinks;
        self.link_health = link_health;
        self.generation
    }
}
