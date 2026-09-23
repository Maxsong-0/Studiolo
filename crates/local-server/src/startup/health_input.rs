//! 组合根装配健康证据来源：**只如实反映本次装配真正拿到了哪些证据**。
//!
//! 健康页的每一节都必须有可指认的证据来源才敢显示"正常"。本轮组合根
//! 一条真实证据源都没有接上，原因逐条写在 [`AssembledEvidence::snapshot`] 上，
//! 因此这里一律给出"未知"。**绝不为了页面好看而假装正常**：
//! `EvidenceSnapshot::unknown` 的每一节都会被只读聚合判成 `Unknown`，
//! 整体因此永远不是绿。
//!
//! 本 Module 不读磁盘、不连网络、不读环境变量，也不持有任何配置或秘密；
//! 唯一的外部输入是本地墙钟，且只在每次取快照时读。

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use studiolo_local_server::health::{EvidenceSnapshot, HealthEvidence};
use studiolo_local_server::sync_plane::health::BridgeHealthProbe;

/// 本次装配可得的健康证据。
///
/// 只要哪天真有证据源接进组合根，就往这里加字段，加一个字段就多一节能显示真实结论；
/// 没加的一律留在"未知"。目前只有双向 Bridge 平面（装配了才有）。
pub(crate) struct AssembledEvidence {
    /// 双向 Bridge 平面的健康探针；平面未装配即 `None`。
    bridge: Option<Arc<dyn BridgeHealthProbe>>,
}

impl HealthEvidence for AssembledEvidence {
    /// 取一次证据快照。
    ///
    /// 逐节说明当前为什么是"未知"：
    ///
    /// - **恢复点年龄**：备份 Worker（T-048 B1）还没接进组合根，本进程既没有
    ///   备份状态句柄也没有清单读取能力，连"有没有恢复点"都读不到，
    ///   因此报"证据不可读"而不是报"尚未建立"——后者是一句我们证明不了的断言。
    ///   等备份 Worker 接线后，它报 `NotEstablished` 时健康页才会显示
    ///   `studiolo_observability::health::NO_RECOVERY_POINT` 那句固定中文。
    /// - **各 owner 活性**：活性回执要 SyncTransportOwner（T-048 B3）与备份
    ///   Worker 各自持久，本轮都没接线，没有任何回执可引用。
    /// - **磁盘配额**：剩余空间只能由持文件系统权限的 Broker 读；
    ///   `vault-broker` 目前没有对外公开的剩余字节查询，HTTP 层自己更不许读盘。
    /// - **Sync 配额**：对象数与字节数属于 Sync 侧账本，本进程拿不到。
    /// - **白名单诊断**：诊断需要把 `SyncScope` / `file_operations` /
    ///   `human_content` 摊平成同一份快照，而 `host-config` 目前只公开
    ///   `observation_selection`（保护前缀与生成区已被合并成一张 `excluded` 表），
    ///   没有公开人侧可写范围与 `human_content` 登记。用半份快照去诊断会凭空
    ///   造出"搬运范围覆盖保护前缀"这类假冲突，比报未知更坏，所以报未知。
    /// - **双向 Bridge**：平面装配了才有探针；第六节、owner 活性与 Sync 配额读数都来自它。
    ///   B 轮 W5 实现：把 `probe.liveness()` 进 `liveness`，`probe.sync_usage()` 配合
    ///   `probe.manual_quota()` 算 `sync`（七成预警），`probe.bridge()` 进 `bridge`。
    fn snapshot(&self) -> EvidenceSnapshot {
        let mut snapshot = EvidenceSnapshot::unknown(now_unix_seconds());
        if let Some(probe) = &self.bridge {
            snapshot.bridge = Some(probe.bridge());
            snapshot.liveness = vec![probe.liveness()];
        }
        snapshot
    }
}

/// 构造本次装配的证据来源。
pub(crate) fn source(bridge: Option<Arc<dyn BridgeHealthProbe>>) -> Arc<dyn HealthEvidence> {
    Arc::new(AssembledEvidence { bridge })
}

/// 本地墙钟的 Unix 秒。
///
/// 时钟早于纪元或超出 `i64` 时返回 `None` 对应的兜底 0：聚合层对"时间证据"
/// 本来就有独立的回退/越界判定，绝不会因为一个可疑时刻而把某节判成正常。
fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{AssembledEvidence, now_unix_seconds, source};
    use studiolo_local_server::health::{HealthBody, HealthEvidence};

    #[test]
    fn 组合根尚未接上任何证据源因而健康页整体判未知而不是判正常() {
        let body = HealthBody::from_report(&AssembledEvidence { bridge: None }.snapshot().report());

        assert_eq!(body.overall, "unknown");
        assert!(!body.evidence_complete);
    }

    #[test]
    fn 六节结论一节不少且全部标为未知() {
        let body = HealthBody::from_report(&AssembledEvidence { bridge: None }.snapshot().report());

        assert_eq!(body.sections.len(), 6);
        assert!(
            body.sections
                .iter()
                .all(|section| section.state == "unknown"),
            "任何一节都不许在没有证据的情况下报正常"
        );
    }

    #[test]
    fn 取到的本地时刻落在纪元之后因而不是兜底零值() {
        assert!(now_unix_seconds() > 1_700_000_000);
    }

    #[test]
    fn 同一个证据来源连续取两次都给出同样的未知结论() {
        let evidence = source(None);

        let first = HealthBody::from_report(&evidence.snapshot().report());
        let second = HealthBody::from_report(&evidence.snapshot().report());

        assert_eq!(first.overall, "unknown");
        assert_eq!(second.overall, "unknown");
        assert_eq!(first.sections.len(), second.sections.len());
    }
}
