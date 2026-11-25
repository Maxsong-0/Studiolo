//! 没认出类型的拒绝登记（T-241）：同一入口同一原因每分钟最多一条，过一分钟再来又有一条；
//! 账本写不进也算用掉这一分钟。时刻由用例给，落到真账本上看。
use super::{Unrecognized, note_unrecognized};
use crate::{options::ServeOptions, serve, state::AppState};
use serde_json::{Value, json};
use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::AgentStore;
use tempfile::TempDir;

include!("../mcp_contract_support/fixture.rs");

/// 账本里的匿名拒绝，按次序：（入口，原因）。
fn recorded(f: &Fixture) -> Vec<(String, String)> {
    f.ledger()
        .query(&AuditFilter::default().with_action("agent.action_denied"))
        .unwrap()
        .into_iter()
        .map(|record| {
            let payload = record.event().payload().as_value().clone();
            assert!(payload["agent_type"].is_null() && payload["agent_instance"].is_null());
            let text = |key: &str| payload[key].as_str().unwrap().to_owned();
            (text("action"), text("reason"))
        })
        .collect()
}

fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(entrance, reason)| ((*entrance).to_owned(), (*reason).to_owned()))
        .collect()
}

#[tokio::test]
async fn each_entrance_and_reason_is_recorded_once_a_minute_and_again_after_it() {
    use Unrecognized::{Unauthenticated, Unavailable};
    let f = fixture().await;
    let start = Instant::now();
    let note = |entrance, refused, millis| {
        note_unrecognized(
            &f.state,
            entrance,
            refused,
            start + Duration::from_millis(millis),
        );
    };
    for millis in [0, 1, 30_000, 59_999] {
        note("mcp.request", Unauthenticated, millis);
    }
    let mut expected = vec![("mcp.request", "unauthenticated")];
    assert_eq!(recorded(&f), pairs(&expected));
    // 另一个原因、另一个入口各算各的，谁也盖不住谁。
    for (entrance, refused, reason) in [
        ("mcp.request", Unavailable, "identity_unavailable"),
        ("http.request", Unauthenticated, "unauthenticated"),
        ("http.request", Unavailable, "identity_unavailable"),
    ] {
        note(entrance, refused, 59_999);
        note(entrance, refused, 59_999);
        expected.push((entrance, reason));
        assert_eq!(recorded(&f), pairs(&expected));
    }
    // 满一分钟再来，又有一条，而且只有一条。
    for millis in [60_000, 60_001, 119_999] {
        note("mcp.request", Unauthenticated, millis);
    }
    expected.push(("mcp.request", "unauthenticated"));
    assert_eq!(recorded(&f), pairs(&expected));
}

#[tokio::test]
async fn a_ledger_that_cannot_be_written_is_tried_once_a_minute_not_once_a_request() {
    let f = fixture().await;
    let start = Instant::now();
    let note = |seconds| {
        note_unrecognized(
            &f.state,
            "mcp.request",
            Unrecognized::Unauthenticated,
            start + Duration::from_secs(seconds),
        );
    };
    let ledger_mode = |mode| {
        fs::set_permissions(f.data.join("audit"), fs::Permissions::from_mode(mode)).unwrap();
    };
    ledger_mode(0o750);
    for _ in 0..5 {
        note(0);
    }
    // 降级窗口照常开，只记这一次试写。
    assert!(f.state.degraded().is_some());
    assert_eq!(f.store().outages().unwrap()[0].3, 1);
    ledger_mode(0o700);
    note(59);
    assert!(recorded(&f).is_empty());
    note(60);
    assert_eq!(recorded(&f), pairs(&[("mcp.request", "unauthenticated")]));
    assert!(f.state.degraded().is_none());
}
