//! 由已派生的本代视图求计数；只读长度与已有的 `Sourced` 值，不重扫文档正文，
//! 也不为了数一个数而把整份文档清单克隆出来。

use crate::{CountRollup, ProjectSummary, WorkbenchCounts};

/// 本代计数。项目基数由已派生的项目切片给出，其余三项只接收长度。
pub(crate) fn counts(
    projects: &[ProjectSummary],
    inbox: usize,
    diagnostics: usize,
    documents: usize,
) -> WorkbenchCounts {
    WorkbenchCounts {
        documents: CountRollup::cardinality(documents, "文档"),
        projects: CountRollup::cardinality(projects.len(), "项目"),
        inbox: CountRollup::cardinality(inbox, "收件箱条目"),
        diagnostics: CountRollup::cardinality(diagnostics, "诊断"),
        active_tasks: CountRollup::rollup(
            projects
                .iter()
                .map(|project| (project.id.as_str(), project.active_tasks.value)),
        ),
        open_bugs: CountRollup::rollup(
            projects
                .iter()
                .map(|project| (project.id.as_str(), project.open_bugs.value)),
        ),
    }
}
