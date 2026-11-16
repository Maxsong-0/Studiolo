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
    /// 可缺失的同步证据明细；不改变任何同步状态。
    pub bridge: Option<super::BridgeBody>,
    /// 人工账号记录，与镜像读数分别显示。
    pub manual_sync_quota: Option<super::ManualQuotaBody>,
    /// 备份作业明细；和「备份作业」一节出自同一次读账本，没读到为 `null`。
    pub backup: Option<super::BackupJobsBody>,
}

impl HealthBody {
    /// 正式端点从同一次快照组合结论与明细。
    #[must_use]
    pub fn from_snapshot(snapshot: &super::EvidenceSnapshot) -> Self {
        let mut body = Self::from_report(&snapshot.report());
        body.bridge = snapshot.bridge.map(super::BridgeBody::from_evidence);
        body.manual_sync_quota = snapshot
            .manual_sync_quota
            .map(|quota| super::ManualQuotaBody::new(quota, snapshot.now_unix_seconds));
        body.backup = match &snapshot.backup_jobs {
            crate::backup::BackupJobsEvidence::Read(jobs) => Some(super::BackupJobsBody::new(jobs)),
            _ => None,
        };
        body
    }

    /// 由只读聚合结果塑形。
    #[must_use]
    pub fn from_report(report: &HealthReport) -> Self {
        Self {
            bridge: None,
            manual_sync_quota: None,
            backup: None,
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
