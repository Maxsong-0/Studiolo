//! 连接器能力集（ADR-046）的校验：只读黑板与推状态两张名单，别的权限必须全空、两张不能同时非空。
//!
//! 「读不到别的」「两样各一张令牌」就在发布这一步由机器保证，不靠提交人自觉。

use std::collections::BTreeSet;

use crate::{model::CapabilitySet, visible::invisible};

use super::{Expansion, Problem, Report, capability::project_path};

/// 项目名在读侧（网页、AI）还要原样过一遍长度和可见性，名单和快照两处都按这一条收。
const MAX_PROJECT_BYTES: usize = 512;

/// 连接器认的项目：完整项目路径，不超长、没有看不见的字符。
#[must_use]
pub fn connector_project(value: &str) -> bool {
    value.len() <= MAX_PROJECT_BYTES && !value.chars().any(invisible) && project_path(value)
}

/// 一张连接器名单在报告里的写法。
struct Roster {
    path: &'static str,
    gained: &'static str,
    lost: &'static str,
}

const BOARD: Roster = Roster {
    path: "read.board_projects",
    gained: "获得该项目黑板的只读权限（列表元数据与单条全文）",
    lost: "收回该项目黑板的只读权限",
};
const PRESENCE: Roster = Roster {
    path: "write.presence_projects",
    gained: "获得推送该项目会话状态的权限",
    lost: "收回推送该项目会话状态的权限",
};

/// 校验两张名单，并报相对现行版本的扩大项与收窄项。
pub(super) fn connector(
    report: &mut Report,
    set: &CapabilitySet,
    previous: Option<&CapabilitySet>,
) {
    roster(
        report,
        &BOARD,
        &set.read.board_projects,
        previous.map(|set| set.read.board_projects.as_slice()),
    );
    roster(
        report,
        &PRESENCE,
        &set.write.presence_projects,
        previous.map(|set| set.write.presence_projects.as_slice()),
    );
    if !set.connector_only() {
        // 名单清空后这一类就成了普通的空能力集，令牌反而开得了 MCP；要停只能撤下或吊销。
        if let Some(previous) = previous.filter(|set| set.connector_only()) {
            let path = if previous.read.board_projects.is_empty() {
                PRESENCE.path
            } else {
                BOARD.path
            };
            report.problems.push(Problem::new(
                path,
                "连接器的名单不能清空，也不能改成普通能力集：要停用就撤下这份能力集，或吊销这个类型",
            ));
        }
        return;
    }
    if !set.read.board_projects.is_empty() && !set.write.presence_projects.is_empty() {
        report.problems.push(Problem::new(
            PRESENCE.path,
            "看黑板和推状态不能在同一份能力集里",
        ));
    }
    let others = [
        (
            "read.project_whitelist",
            !set.read.project_whitelist.is_empty(),
        ),
        ("read.rules_whitelist", !set.read.rules_whitelist.is_empty()),
        ("write.board", set.write.board),
        ("write.board_projects", !set.write.board_projects.is_empty()),
        ("write.content_patch_only", set.write.content_patch_only),
        ("policy.propose_desired", set.policy.propose_desired),
        ("hard_bound_skills", !set.hard_bound_skills.is_empty()),
    ];
    for (path, granted) in others {
        if granted {
            report
                .problems
                .push(Problem::new(path, "连接器能力集里别的权限必须全空"));
        }
    }
}

fn roster(report: &mut Report, roster: &Roster, next: &[String], previous: Option<&[String]>) {
    let mut seen = BTreeSet::new();
    for (index, project) in next.iter().enumerate() {
        let path = format!("{}[{index}]", roster.path);
        if !connector_project(project) {
            report
                .problems
                .push(Problem::new(path, "必须是合法的完整项目路径"));
        } else if !seen.insert(project.as_str()) {
            report.problems.push(Problem::new(path, "重复的项目"));
        }
    }
    let old = previous.unwrap_or_default();
    for project in next.iter().filter(|project| !old.contains(project)) {
        report.expansions.push(Expansion {
            path: roster.path.into(),
            before: None,
            after: Some(project.clone()),
            note: roster.gained.into(),
        });
    }
    for project in old.iter().filter(|project| !next.contains(project)) {
        report.narrowings.push(Expansion {
            path: roster.path.into(),
            before: Some(project.clone()),
            after: None,
            note: roster.lost.into(),
        });
    }
}
