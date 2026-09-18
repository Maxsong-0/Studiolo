//! 健康页 HTTP 层的行为／失败／恢复三类契约。
//!
//! 这里只覆盖本 Module 自己能独立验收的部分：门禁前置、响应塑形与证据缺失兜底。
//! 真实装配后的端到端鉴权由组合根的整套契约覆盖。

use super::{EvidenceSnapshot, HealthBody, PATH, enabled, routes};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studiolo_observability::health::{
    Instant, OwnerLabel, OwnerLiveness, RecoveryPointEvidence, WhitelistSnapshot, diagnose,
};
use studiolo_observability::quota::QuotaVerdict;
use tower::ServiceExt;

fn complete(now: i64) -> EvidenceSnapshot {
    EvidenceSnapshot {
        now_unix_seconds: now,
        recovery_point: Some(RecoveryPointEvidence::Established {
            content_cutoff: Instant::from_unix_seconds(now - 100),
        }),
        recovery_point_budget_seconds: 3_600,
        liveness: vec![OwnerLiveness {
            owner: OwnerLabel::parse("backup-worker").expect("固定标签必须合法"),
            last_receipt: Some(Instant::from_unix_seconds(now - 10)),
            expected_period_seconds: 600,
        }],
        disk: QuotaVerdict::Normal,
        sync: QuotaVerdict::Normal,
        whitelist: Some(diagnose(&WhitelistSnapshot::default()).expect("空白名单必须可诊断")),
    }
}

#[test]
fn 健康页路径落在api前缀下因而自动走既有鉴权门禁() {
    assert!(PATH.starts_with("/api/"));
}

#[test]
fn 没有会话门禁时一律不挂载健康页以免新增无鉴权入口() {
    assert!(!enabled(false, true));
    assert!(!enabled(false, false));
}

#[test]
fn 没有证据来源时也不挂载健康页() {
    assert!(!enabled(true, false));
}

#[test]
fn 会话与证据都具备时才挂载健康页() {
    assert!(enabled(true, true));
}

#[derive(Debug)]
struct FixedEvidence(i64);

impl super::HealthEvidence for FixedEvidence {
    fn snapshot(&self) -> EvidenceSnapshot {
        complete(self.0)
    }
}

/// 一会儿读不到证据、一会儿又读得到的来源，用来验收"恢复"契约。
struct FlakyEvidence {
    now: i64,
    readable: AtomicBool,
}

impl super::HealthEvidence for FlakyEvidence {
    fn snapshot(&self) -> EvidenceSnapshot {
        if self.readable.load(Ordering::SeqCst) {
            complete(self.now)
        } else {
            EvidenceSnapshot::unknown(self.now)
        }
    }
}

fn source(evidence: impl super::HealthEvidence + 'static) -> Arc<dyn super::HealthEvidence> {
    Arc::new(evidence)
}

/// 装出一套"已装会话门禁"的最小路由；证据来源按参数决定装不装。
fn served(evidence: Option<Arc<dyn super::HealthEvidence>>) -> Router {
    let router: Router = routes(Router::new(), true);
    match evidence {
        Some(evidence) => super::with_health_evidence(router, evidence),
        None => router,
    }
}

/// 发一次匿名 GET。此处**没有**门禁中间件，所以只能验收装配与塑形；
/// 真实入口的鉴权由 `security::guard` 对 `/api/` 前缀强制执行，
/// 端到端证据见 `crates/local-server/tests/auth_state/`。
async fn get(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
    let Ok(request) = Request::builder().uri(path).body(Body::empty()) else {
        return (StatusCode::BAD_REQUEST, serde_json::Value::Null);
    };
    // `Router` 的错误类型是 `Infallible`，这里用类型系统消掉而不是 unwrap。
    let response = app
        .oneshot(request)
        .await
        .unwrap_or_else(|never| match never {});
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

#[test]
fn 没有会话门禁时装配函数原样返回不挂载任何路由() {
    let router: Router<()> = routes(Router::new(), false);

    assert!(!router.has_routes());
}

#[test]
fn 会话门禁具备时装配函数挂载健康页路由() {
    let router: Router<()> = routes(Router::new(), true);

    assert!(router.has_routes());
}

#[tokio::test]
async fn 装配了证据来源时健康页给出封闭json且五节齐全() {
    let (status, body) = get(served(Some(source(FixedEvidence(10_000)))), PATH).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["overall"], "normal");
    assert_eq!(body["evidence_complete"], true);
    assert_eq!(
        body["sections"].as_array().map(Vec::len),
        Some(5),
        "五节必须齐全"
    );
}

#[tokio::test]
async fn 没有装配证据来源时健康页回404因而等同于不存在该入口() {
    let (status, body) = get(served(None), PATH).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "本机未装配健康证据来源。");
    assert!(body.get("overall").is_none(), "缺证据时不得给出任何结论");
}

#[tokio::test]
async fn 证据来源读不到任何证据时健康页整体未知且不标记证据完整() {
    let flaky: Arc<dyn super::HealthEvidence> = Arc::new(FlakyEvidence {
        now: 10_000,
        readable: AtomicBool::new(false),
    });

    let (status, body) = get(served(Some(flaky)), PATH).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["overall"], "unknown");
    assert_eq!(body["evidence_complete"], false);
}

#[tokio::test]
async fn 证据一度读不到之后重新可得时健康页自行恢复成正常() {
    let flaky = Arc::new(FlakyEvidence {
        now: 10_000,
        readable: AtomicBool::new(false),
    });
    let installed: Arc<dyn super::HealthEvidence> = Arc::clone(&flaky) as _;
    let (_, degraded) = get(served(Some(Arc::clone(&installed))), PATH).await;
    assert_eq!(degraded["overall"], "unknown");

    flaky.readable.store(true, Ordering::SeqCst);
    let (status, recovered) = get(served(Some(installed)), PATH).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(recovered["overall"], "normal");
    assert_eq!(recovered["evidence_complete"], true);
}

#[test]
fn 证据齐备时响应标记证据完整并给出全部五节() {
    let body = HealthBody::from_report(&complete(10_000).report());

    assert_eq!(body.overall, "normal");
    assert!(body.evidence_complete);
    assert_eq!(body.sections.len(), 5);
}

#[test]
fn 证据全部缺失时响应整体未知且不标记证据完整() {
    let body = HealthBody::from_report(&EvidenceSnapshot::unknown(10_000).report());

    assert_eq!(body.overall, "unknown");
    assert!(!body.evidence_complete);
    assert!(
        body.sections
            .iter()
            .all(|section| section.state == "unknown")
    );
}

#[test]
fn 未建立恢复点时响应不显示绿并且带既定中文() {
    let mut snapshot = complete(10_000);
    snapshot.recovery_point = Some(RecoveryPointEvidence::NotEstablished);

    let body = HealthBody::from_report(&snapshot.report());

    assert_ne!(body.overall, "normal");
    let section = body
        .sections
        .iter()
        .find(|section| section.code == "recovery-point")
        .expect("必须包含恢复点一节");
    assert_eq!(section.detail, "尚未建立灾难恢复点");
}

#[test]
fn 响应正文不含路径邮箱形状或控制字符() {
    let body = HealthBody::from_report(&complete(10_000).report());

    for section in &body.sections {
        assert!(!section.detail.contains('/'));
        assert!(!section.detail.contains('@'));
        assert!(!section.detail.chars().any(char::is_control));
    }
}

#[test]
fn 证据从全部缺失恢复成齐备之后响应重新显示正常() {
    let degraded = HealthBody::from_report(&EvidenceSnapshot::unknown(10_000).report());
    assert_eq!(degraded.overall, "unknown");

    let recovered = HealthBody::from_report(&complete(10_000).report());

    assert_eq!(recovered.overall, "normal");
    assert!(recovered.evidence_complete);
}

#[test]
fn 响应可以序列化成封闭json且字段固定() {
    let body = HealthBody::from_report(&complete(10_000).report());

    let text = serde_json::to_string(&body).expect("响应必须可序列化");

    assert!(text.contains("\"overall\":\"normal\""));
    assert!(text.contains("\"evidence_complete\":true"));
    assert!(text.contains("\"code\":\"recovery-point\""));
}
