//! 探针接线契约：`BridgeHealthProbe` 给出的第六节与 `sync-transport` 活性原样进入
//! `EvidenceSnapshot`，缺证据时两节判未知；响应里第六节的机器码与标题固定。

use super::{complete, normal_bridge};
use crate::health::{EvidenceSnapshot, HealthBody, SectionBody};
use crate::sync_plane::health::{BridgeHealthProbe, OWNER_LABEL};
use studiolo_host_config::sync_transport::ManualQuota;
use studiolo_observability::health::{BridgeEvidence, Instant, OwnerLabel, OwnerLiveness};
use studiolo_observability::quota::SyncUsage;

/// 可控探针：`receipt` 为 `None` 时活性无回执，`evidence` 为默认值时第六节全缺。
struct FakeProbe {
    evidence: BridgeEvidence,
    receipt: Option<i64>,
}

impl BridgeHealthProbe for FakeProbe {
    fn bridge(&self) -> BridgeEvidence {
        self.evidence
    }

    fn liveness(&self) -> OwnerLiveness {
        OwnerLiveness {
            owner: OwnerLabel::parse(OWNER_LABEL).expect("固定标签必须合法"),
            last_receipt: self.receipt.map(Instant::from_unix_seconds),
            expected_period_seconds: 600,
        }
    }

    fn sync_usage(&self) -> Option<SyncUsage> {
        None
    }

    fn manual_quota(&self) -> Option<ManualQuota> {
        None
    }
}

/// 照 `startup/health_input.rs` 的接线方式把探针读数放进快照；其余各节用齐备夹具。
fn snapshot_from(probe: &dyn BridgeHealthProbe, now: i64) -> EvidenceSnapshot {
    let mut snapshot = complete(now);
    snapshot.bridge = Some(probe.bridge());
    snapshot.liveness = vec![probe.liveness()];
    snapshot
}

/// 用给定探针读数渲染一次响应体；其余各节取 `complete(10_000)` 的齐备夹具。
fn render(evidence: BridgeEvidence, receipt: Option<i64>) -> HealthBody {
    let probe = FakeProbe { evidence, receipt };
    HealthBody::from_report(&snapshot_from(&probe, 10_000).report())
}

fn section<'a>(body: &'a HealthBody, code: &str) -> &'a SectionBody {
    body.sections
        .iter()
        .find(|section| section.code == code)
        .expect("响应必须包含该节")
}

#[test]
fn 探针证据齐备时第六节与传输活性都显示正常() {
    let body = render(normal_bridge(10_000), Some(9_990));

    let bridge = section(&body, "sync-bridge");
    assert_eq!(bridge.title, "双向 Bridge");
    assert_eq!(bridge.state, "normal");
    assert!(bridge.detail.starts_with("第 3 批于 9970 闭合"));
    let liveness = section(&body, "owner-liveness");
    assert_eq!(liveness.state, "normal");
    assert_eq!(
        liveness.detail,
        "活性回执正常 1 个，超期 0 个，证据不足 0 个"
    );
    assert_eq!(body.overall, "normal");
}

#[test]
fn 探针没有任何证据时第六节与传输活性判未知而整体不显示绿() {
    let body = render(BridgeEvidence::default(), None);

    assert_eq!(section(&body, "sync-bridge").state, "unknown");
    assert_eq!(section(&body, "owner-liveness").state, "unknown");
    assert_eq!(body.overall, "unknown");
    assert!(!body.evidence_complete);
}

#[test]
fn 传输活性回执超期时活性节降级并压过第六节的正常() {
    let body = render(normal_bridge(10_000), Some(9_000));

    assert_eq!(section(&body, "owner-liveness").state, "degraded");
    assert_eq!(section(&body, "sync-bridge").state, "normal");
    assert_eq!(body.overall, "degraded");
}

#[test]
fn 响应里第六节的机器码与标题固定() {
    let body = render(normal_bridge(10_000), Some(9_990));

    let text = serde_json::to_string(&body).expect("响应必须可序列化");

    assert!(text.contains("\"code\":\"sync-bridge\""));
    assert!(text.contains("\"title\":\"双向 Bridge\""));
}
