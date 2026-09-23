//! 每个真实 MCP 请求验证副本并返回同批来源信息，故障与审计均失败关闭。
#[path = "../../supply/tests/replica_support/mod.rs"]
mod replica_support;
#[path = "replica_http/support.rs"]
mod support;

use replica_support::{SECOND, write};
use serde_json::json;
use std::{fs, time::Instant};
use studiolo_audit_ledger::{AuditFilter, AuditLedger};
use studiolo_supply::{HARD_BUDGET_TOKENS, count_tokens};
use support::{fixture, is_error, tool_text};

#[tokio::test]
async fn all_four_tools_follow_one_verified_generation_and_never_mix_raw_file_reads() {
    let f = fixture().await;
    let started = Instant::now();
    let (session, init) = f.initialize(&f.token).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("状态 ready") && instructions.contains("oldmarker"));
    assert!(count_tokens(instructions) <= HARD_BUDGET_TOKENS);
    f.replica
        .publish(SECOND, "newmarker", f.replica.observed + 1000);
    for (index, (tool, args)) in [
        ("handshake", json!({})),
        ("list_entries", json!({"project": "project"})),
        ("read_note", json!({"path": "project/note.md"})),
        ("search", json!({"project": "project", "text": "newmarker"})),
    ]
    .into_iter()
    .enumerate()
    {
        let response = f
            .call(&f.token, &session, index as u64 + 2, tool, args)
            .await;
        assert!(!is_error(&response), "{tool}: {response}");
        let text = tool_text(&response);
        assert!(
            text.contains(&format!("generation={SECOND}")),
            "{tool}: {text}"
        );
        assert!(!text.contains("oldmarker"));
        if tool != "list_entries" {
            assert!(text.contains("newmarker"), "{tool}: {text}");
        }
    }
    let reads = f
        .store()
        .query_reads(&studiolo_supply::ReadQuery {
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert!(reads.iter().any(|read| read.tool == "read_note"));
    eprintln!(
        "replica loopback initialize + four verified tools: {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn drift_and_invalid_control_refuse_all_tools_and_audit_failure_takes_precedence() {
    let f = fixture().await;
    let (session, _) = f.initialize(&f.token).await;
    write(&f.replica.file("project/note.md"), b"# injected secret");
    for (index, (tool, args)) in [
        ("handshake", json!({})),
        ("list_entries", json!({"project": "project"})),
        ("read_note", json!({"path": "project/note.md"})),
        ("search", json!({"project": "project", "text": "oldmarker"})),
    ]
    .into_iter()
    .enumerate()
    {
        let response = f
            .call(&f.token, &session, index as u64 + 2, tool, args)
            .await;
        assert!(is_error(&response), "{tool}: {response}");
        assert!(response.to_string().contains("replica_drift"));
        assert!(!response.to_string().contains("injected secret"));
    }
    assert_eq!(
        f.ledger()
            .query(&AuditFilter::default().with_action("agent.action_denied"))
            .unwrap()
            .len(),
        4
    );
    write(&f.replica.root.join("control/state.json"), b"{}");
    let response = f.call(&f.token, &session, 8, "handshake", json!({})).await;
    assert!(is_error(&response) && response.to_string().contains("replica_invalid"));
    // 既有账本保存了根绑定：即使它持有的目录句柄仍然有效，也能识别出目录已被替换。
    fs::rename(f.data.join("audit"), f.data.join("audit-away")).unwrap();
    fs::create_dir(f.data.join("audit")).unwrap();
    let response = f.call(&f.token, &session, 9, "handshake", json!({})).await;
    assert!(
        is_error(&response) && response.to_string().contains("审计不可用"),
        "{response}"
    );
    assert!(!response.to_string().contains("replica_invalid"));
}

#[tokio::test]
async fn stale_and_sync_failure_are_explicit_without_fabricating_a_new_source_time() {
    let f = support::fixture_with_age(400_000).await;
    let (session, init) = f.initialize(&f.token).await;
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("状态 stale") && instructions.contains("oldmarker"));
    assert!(count_tokens(instructions) <= HARD_BUDGET_TOKENS);
    let mut state = f.replica.state();
    let old = state["receipt"]["source_observed_at_ms"].as_i64().unwrap();
    state["status"] = json!("failed");
    state["last_error"] = json!("transfer_failed");
    state["last_attempt_at_ms"] = json!(jiff::Timestamp::now().as_millisecond());
    f.replica.write_state(&state);
    for (index, (tool, args)) in [
        ("handshake", json!({})),
        ("list_entries", json!({"project": "project"})),
        ("read_note", json!({"path": "project/note.md"})),
        ("search", json!({"project": "project", "text": "oldmarker"})),
    ]
    .into_iter()
    .enumerate()
    {
        let response = f
            .call(&f.token, &session, index as u64 + 2, tool, args)
            .await;
        assert!(!is_error(&response), "{tool}: {response}");
        let status = tool_text(&response);
        assert!(status.contains("sync_failed") && status.contains("stale=true"));
        assert!(status.contains(&format!("source_observed_at_ms={old}")));
    }
}

#[test]
fn replica_options_are_explicit_and_mutually_exclusive() {
    use studiolo_agent_server::options::{Command, parse};
    let args = [
        "serve",
        "--replica-root",
        "/r",
        "--allow",
        "project",
        "--data",
        "/d",
    ];
    let strings = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let parsed = parse(&strings(&args)).unwrap();
    assert!(
        matches!(parsed, Command::Serve(options) if options.vault.is_none() && options.replica_max_age_secs == 300)
    );
    for extra in [
        ["--vault", "/v"],
        ["--replica-max-age-secs", "59"],
        ["--replica-max-age-secs", "86401"],
    ] {
        let mut full = args.to_vec();
        full.extend(extra);
        assert!(parse(&strings(&full)).is_err());
    }
}

#[tokio::test]
async fn concurrent_rescans_are_serial_and_cannot_overwrite_a_later_generation() {
    let f = fixture().await;
    let initial = f.state.current().number;
    let mut scans = Vec::new();
    for _ in 0..8 {
        let state = f.state.clone();
        scans.push(tokio::task::spawn_blocking(move || state.rescan().unwrap()));
    }
    let mut numbers = Vec::new();
    for scan in scans {
        numbers.push(scan.await.unwrap());
    }
    numbers.sort_unstable();
    assert_eq!(numbers, (initial + 1..=initial + 8).collect::<Vec<_>>());
    assert_eq!(f.state.current().number, initial + 8);
}
