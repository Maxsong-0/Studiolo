//! 按逻辑路径前缀取有序区间；派生每个项目时只访问该项目自己的文档，
//! 不为每个项目重扫全库。

use std::collections::BTreeMap;
use std::ops::Bound;

/// 前缀区间内的条目，按键有序。访问次数等于命中条目数加一次越界判定，
/// 与库内其余文档数无关。
pub(crate) fn under<'a, V>(
    documents: &'a BTreeMap<String, V>,
    prefix: &'a str,
) -> impl Iterator<Item = (&'a String, &'a V)> {
    documents
        .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
        .take_while(move |(path, _)| path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::under;
    use std::collections::BTreeMap;

    fn vault() -> BTreeMap<String, usize> {
        let mut documents = BTreeMap::new();
        for index in 0..500 {
            documents.insert(format!("笔记/{index:04}.md"), index);
        }
        for name in ["BUG-001.md", "BUG-002.md", "归档/BUG-旧.md"] {
            documents.insert(format!("项目/甲/06-BUG/{name}"), 0);
        }
        for index in 0..500 {
            documents.insert(format!("项目/乙/06-BUG/BUG-{index:04}.md"), index);
        }
        documents
    }

    #[test]
    fn prefix_scan_visits_only_the_entries_inside_the_prefix() {
        let documents = vault();
        let visited: Vec<_> = under(&documents, "项目/甲/06-BUG/")
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(
            visited,
            [
                "项目/甲/06-BUG/BUG-001.md",
                "项目/甲/06-BUG/BUG-002.md",
                "项目/甲/06-BUG/归档/BUG-旧.md",
            ]
        );
        assert_eq!(documents.len(), 1003);
    }

    #[test]
    fn prefix_scan_of_an_absent_prefix_visits_nothing() {
        let documents = vault();
        assert_eq!(under(&documents, "项目/丙/06-BUG/").count(), 0);
    }
}
