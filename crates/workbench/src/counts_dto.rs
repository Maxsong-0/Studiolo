//! 只给计数、不给整份文档清单的投影值类型；计数不可得时显式说明原因。

use serde::Serialize;

use crate::{ActionItem, ProjectSummary};

/// 一个计数及其不可得的显式原因。
///
/// `value` 为 `None` 表示计数本身不可得，绝不退化成 `0`；`unavailable` 必定非空并
/// 逐条说明是哪个来源使计数不可得，便于界面直接展示而不是编造一个数字。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CountRollup {
    /// 已确认的计数；`None` 表示不可得。
    pub value: Option<u64>,
    /// 使计数不可得的来源说明，按来源标识的码位序排序去重；计数可得时为空。
    pub unavailable: Vec<String>,
}

impl CountRollup {
    /// 一个确定可得的基数，超出 64 位表示范围时显式转为不可得。
    pub(crate) fn cardinality(count: usize, subject: &str) -> Self {
        u64::try_from(count).map_or_else(
            |_| Self {
                value: None,
                unavailable: vec![format!("{subject}总数超出 64 位计数范围")],
            },
            |value| Self {
                value: Some(value),
                unavailable: Vec::new(),
            },
        )
    }

    /// 逐项累加；任何一项不可得都让整个汇总不可得，不做部分求和。
    pub(crate) fn rollup<'a>(items: impl Iterator<Item = (&'a str, Option<u64>)>) -> Self {
        let mut total = Some(0);
        let mut unavailable = Vec::new();
        for (subject, value) in items {
            let Some(value) = value else {
                unavailable.push(subject.to_owned());
                continue;
            };
            total = total.and_then(|sum: u64| sum.checked_add(value));
            if total.is_none() {
                unavailable.push("累加结果超出 64 位计数范围".to_owned());
            }
        }
        unavailable.sort();
        unavailable.dedup();
        if unavailable.is_empty() {
            Self {
                value: total,
                unavailable,
            }
        } else {
            Self {
                value: None,
                unavailable,
            }
        }
    }
}

/// 一代视图的全部计数；构造它不遍历文档正文，也不复制文档清单。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WorkbenchCounts {
    /// 本代 Markdown 文档总数。
    pub documents: CountRollup,
    /// 本代识别出的项目总数。
    pub projects: CountRollup,
    /// 本代收件箱条目总数。
    pub inbox: CountRollup,
    /// 本代代次级诊断条目总数。
    pub diagnostics: CountRollup,
    /// 各项目进行中任务数之和；任一项目不可得则整体不可得。
    pub active_tasks: CountRollup,
    /// 各项目未关闭 BUG 数之和；任一项目不可得则整体不可得。
    pub open_bugs: CountRollup,
}

/// 首屏投影：项目、收件箱、诊断与计数，不含逐篇文档清单。
///
/// 与 `WorkbenchSnapshot` 的唯一差别就是不携带 `documents`：文档改由 `counts.documents`
/// 表示，逐篇内容走文件树分页与搜索，不再随首屏整份 JSON 化。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WorkbenchOverview {
    /// 按逻辑根排序的项目。
    pub projects: Vec<ProjectSummary>,
    /// 按项目、来源路径、行号与类型排序的收件箱条目。
    pub inbox: Vec<ActionItem>,
    /// 代次级解析提示。
    pub diagnostics: Vec<String>,
    /// 本代计数，含不可得说明。
    pub counts: WorkbenchCounts,
}

#[cfg(test)]
mod tests {
    use super::CountRollup;

    #[test]
    fn a_rollup_of_known_values_sums_them_without_any_unavailable_note() {
        let rollup = CountRollup::rollup([("甲", Some(2)), ("乙", Some(3))].into_iter());
        assert_eq!(rollup.value, Some(5));
        assert!(rollup.unavailable.is_empty());
    }

    #[test]
    fn one_unavailable_item_makes_the_whole_rollup_unavailable_instead_of_partially_summing() {
        let rollup = CountRollup::rollup([("甲", Some(2)), ("乙", None)].into_iter());
        assert_eq!(rollup.value, None, "不做部分求和，不退化成 2");
        assert_eq!(rollup.unavailable, ["乙"]);
    }

    #[test]
    fn a_sum_that_overflows_sixty_four_bits_is_unavailable_and_says_so_exactly_once() {
        let rollup = CountRollup::rollup(
            [("甲", Some(u64::MAX)), ("乙", Some(1)), ("丙", Some(1))].into_iter(),
        );
        assert_eq!(rollup.value, None);
        assert_eq!(rollup.unavailable, ["累加结果超出 64 位计数范围"]);
    }

    #[test]
    fn unavailable_sources_are_sorted_by_code_point_and_never_repeated() {
        let rollup = CountRollup::rollup([("乙", None), ("甲", None), ("乙", None)].into_iter());
        assert_eq!(rollup.value, None);
        assert_eq!(rollup.unavailable, ["乙", "甲"]);
    }

    #[test]
    fn a_cardinality_that_fits_is_available_with_no_note() {
        let rollup = CountRollup::cardinality(7, "文档");
        assert_eq!(rollup.value, Some(7));
        assert!(rollup.unavailable.is_empty());
    }
}
