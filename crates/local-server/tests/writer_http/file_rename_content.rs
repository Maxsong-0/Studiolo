//! 普通坏 UTF-8 会被首次捕获/新鲜度挡住；此处在已取得 epoch 后定点测试完整 admission。
use super::{TO, committed, rename, support, unchanged};
use crate::{file_operations::post, fixture::Fixture};
use std::{fs, sync::Arc};
use studiolo_audit_ledger::AuditPhase;
use studiolo_core::{Revision, VaultPath};

#[tokio::test]
async fn valid_empty_and_text_bytes_are_preserved_without_normalization() {
    for bytes in [
        Vec::new(),
        "# 中文\r\n\t文字\u{feff}\n".as_bytes().to_vec(),
        vec![b'x'; 4096],
    ] {
        let f = Fixture::file_rename(None, &bytes, false).await;
        let issued = f.http.issue().await;
        committed(
            &f,
            &post(&f, &issued, &rename(&f, "valid-text-rename-001"), 200).await,
        );
        f.close().await;
    }
}

#[tokio::test]
async fn complete_invalid_utf8_and_control_bytes_fail_admission_without_a_typed_mutation() {
    let mut middle = vec![b'x'; 1500];
    middle[800] = 0xff;
    let mut trailing = vec![b'x'; 1500];
    trailing.extend_from_slice(&[0xe4, 0xb8]);
    let mut cases: Vec<_> = [
        middle,
        trailing,
        b"abc\0def".to_vec(),
        b"abc\x01def".to_vec(),
        b"abc\x7fdef".to_vec(),
        "abc\u{0085}def".as_bytes().to_vec(),
        b"%PDF-1.7\n\0binary".to_vec(),
    ]
    .into_iter()
    .map(|bytes| (bytes, 415, "unsupported_rename_content"))
    .collect();
    // 纯有效文本只因实际 reader 消费 pair 上限而拒绝，与 415 文本语言分开。
    cases.push((vec![b'x'; 4097], 413, "too_large"));
    for (bytes, status, code) in cases {
        // 首次真实 view 仍必须完整有效；不改生产捕获或 reader 语义。
        let mut f = Fixture::file_rename(None, b"initial valid text\n", false).await;
        let clock = Arc::new(support::AdmissionClock::new(f.http.now()));
        support::clock(&mut f, clock.clone()).await;
        let issued = f.http.issue().await;
        let mut input = rename(&f, "invalid-text-rename-001");
        // 请求确切 expected 来自待替换的真实 raw bytes，不能因 revision 不符绕过分类。
        input["expected_revision"] = Revision::compute(
            f.expected.vault_uuid,
            &VaultPath::parse("a.md").unwrap(),
            &bytes,
        )
        .to_hex()
        .into();
        let before = f.journal();
        clock.replace(f.http.root.join("vault/a.md"), bytes.clone());
        let response = post(&f, &issued, &input, status).await;
        assert_eq!(clock.calls(), 3, "必须命中 epoch 后 admission 中的 verify");
        clock.disarm();
        assert_eq!(response["code"], code);
        assert!(response.get("receipt").is_none() && response.get("content").is_none());
        unchanged(&f, &before, &bytes);
        f.close().await;
    }
}

#[tokio::test]
async fn fixed_rejected_final_session_expiry_is_known_and_original_key_remains_rejected() {
    let mut f = Fixture::file_rename(None, b"unchanged original bytes\n", false).await;
    let clock = Arc::new(support::AdmissionClock::new(f.http.now()));
    support::clock(&mut f, clock.clone()).await;
    let issued = f.http.issue().await;
    let mut input = rename(&f, "fixed-rejected-final-gate");
    input["expected_revision"] = Revision::compute(
        f.expected.vault_uuid,
        &VaultPath::parse("a.md").unwrap(),
        b"different original revision",
    )
    .to_hex()
    .into();
    // 不伪造 Rejected：Broker 真实源 CAS 固定它，然后在最终验证使真实会话到期。
    clock.expire(f.http.now() + 1_801_000);
    let response = post(&f, &issued, &input, 401).await;
    assert_eq!(clock.calls(), 7, "必须在 Intent ACK 后最后 verify 到期");
    clock.disarm();
    assert_eq!(response["code"], "session_required");
    assert!(response.get("receipt").is_none());
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    assert!(!f.http.root.join("vault").join(TO).exists());
    assert_eq!(f.journal()["committed_through"], 0);
    assert!(
        f.journal()["file_operation"].is_null(),
        "两个固定事件均 ACK 后才能收口"
    );
    let records = f.http.events("vault.mutation");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    let outcome = records[1].event().payload().as_value();
    assert_eq!(outcome["file_operation"]["outcome"], "rejected");
    // 同步可信测试时钟到 Auth 已持久时间地板，用新的真实会话核对原主体和原键。
    f.http.advance(1_801_000);
    let renewed = f.http.issue().await;
    assert_eq!(
        post(&f, &renewed, &input, 409).await["code"],
        "file_operation_conflict"
    );
    assert_eq!(f.http.events("vault.mutation").len(), 2);
    assert_eq!(
        fs::read(f.http.root.join("vault/a.md")).unwrap(),
        f.expected.content
    );
    assert!(!f.http.root.join("vault").join(TO).exists());
    f.close().await;
}
