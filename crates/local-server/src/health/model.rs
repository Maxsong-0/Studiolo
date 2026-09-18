//! 健康页 HTTP 响应的纯塑形：把只读聚合结果映射成封闭 JSON。
//!
//! 正文由 `studiolo-observability` 在构造期排除路径、邮箱形状与控制字符；
//! 这里只做搬运，不新增任何来自配置或文件系统的字段。

use serde::Serialize;
use studiolo_observability::health::{HealthReport, HealthState};

/// 一节健康结论的对外表示。
#[derive(Debug, Serialize)]
pub struct SectionBody {
    /// 稳定机器码。
    pub code: &'static str,
    /// 中文标题。
    pub title: &'static str,
    /// 状态机器码：`normal` / `degraded` / `unknown`。
    pub state: &'static str,
    /// 中文结论；不含路径、主人身份或秘密。
    pub detail: String,
}

/// 健康页响应体。
#[derive(Debug, Serialize)]
pub struct HealthBody {
    /// 整体状态机器码。
    pub overall: &'static str,
    /// 证据是否完整；`false` 时页面只应显示"未知"，不得显示绿。
    pub evidence_complete: bool,
    /// 各节结论。
    pub sections: Vec<SectionBody>,
}

impl HealthBody {
    /// 由只读聚合结果塑形。
    #[must_use]
    pub fn from_report(report: &HealthReport) -> Self {
        Self {
            overall: report.overall().code(),
            evidence_complete: report
                .sections()
                .iter()
                .all(|section| section.state() != HealthState::Unknown),
            sections: report
                .sections()
                .iter()
                .map(|section| SectionBody {
                    code: section.code(),
                    title: section.title(),
                    state: section.state().code(),
                    detail: section.detail().to_owned(),
                })
                .collect(),
        }
    }
}
