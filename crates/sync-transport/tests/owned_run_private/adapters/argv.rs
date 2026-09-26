//! 封闭 argv 与 IPC 帧闭集的补充契约（T-132 P1）：从 `adapters.rs` 拆出只为压住单文件行数。

use super::super::support::*;
use crate::linux::ipc::ALLOWED_KEYS;
use crate::linux::protocol::ALL_KEYS;
use crate::linux::service::{argv_digest, build_argv};
use crate::owned_run::{RunOperation, SyncMode};
use std::path::Path;

#[test]
fn the_set_mode_argv_ends_with_the_mode_flag_and_json_and_opens_nothing_else() {
    let approval = FakeApproval::new();
    let mirror = Path::new(MIRROR_ROOT);
    for (mode, literal) in [
        (SyncMode::MirrorRemote, "mirror-remote"),
        (SyncMode::Bidirectional, "bidirectional"),
    ] {
        let argv =
            build_argv(&approval.runtime, RunOperation::SetMode(mode), mirror).expect("可以构造");
        assert_eq!(
            &argv[..2],
            &[
                "/opt/studiolo/node/bin/node",
                "/opt/studiolo/obsidian-headless/cli.js"
            ]
        );
        assert_eq!(
            &argv[2..],
            &[
                "sync-config",
                "--path",
                MIRROR_ROOT,
                "--mode",
                literal,
                "--json"
            ]
        );
        assert_eq!(SyncMode::from_cli_value(literal), Some(mode));
        assert_eq!(RunOperation::SetMode(mode).name(), "set_mode");
    }
    assert_eq!(SyncMode::from_cli_value("continuous"), None);
}

#[test]
fn the_argv_digest_is_stable_domain_separated_and_sensitive_to_item_boundaries() {
    let approval = FakeApproval::new();
    let mirror = Path::new(MIRROR_ROOT);
    let inspect =
        build_argv(&approval.runtime, RunOperation::InspectConfig, mirror).expect("可以构造");
    let once = build_argv(&approval.runtime, RunOperation::RunOnce, mirror).expect("可以构造");
    let set_mode = build_argv(
        &approval.runtime,
        RunOperation::SetMode(SyncMode::MirrorRemote),
        mirror,
    )
    .expect("可以构造");
    assert_eq!(argv_digest(&inspect), argv_digest(&inspect));
    assert_ne!(argv_digest(&inspect), argv_digest(&once));
    assert_ne!(argv_digest(&inspect), argv_digest(&set_mode));
    assert_ne!(argv_digest(&once), argv_digest(&set_mode));
    // 逐项长度前缀：拼接相同但分段不同、或顺序不同的 argv，摘要都不同。
    let split = |items: &[&str]| {
        items
            .iter()
            .map(|item| (*item).to_owned())
            .collect::<Vec<_>>()
    };
    assert_ne!(
        argv_digest(&split(&["ab", "c"])),
        argv_digest(&split(&["a", "bc"]))
    );
    assert_ne!(
        argv_digest(&split(&["a", "b"])),
        argv_digest(&split(&["b", "a"]))
    );
    // 域分隔：不是任何一种裸拼接的 sha256。
    use sha2::{Digest, Sha256};
    let bare: [u8; 32] = Sha256::digest(inspect.concat().as_bytes()).into();
    assert_ne!(argv_digest(&inspect), bare);
    let joined: [u8; 32] = Sha256::digest(inspect.join(" ").as_bytes()).into();
    assert_ne!(argv_digest(&inspect), joined);
}

#[test]
fn every_protocol_key_is_in_the_closed_frame_set_and_no_executable_or_secret_key_is() {
    for key in ALL_KEYS {
        assert!(
            ALLOWED_KEYS.contains(&key),
            "{key} 不在 ipc::ALLOWED_KEYS 里"
        );
    }
    for forbidden in [
        "argv",
        "mirror_path",
        "node_path",
        "cli_path",
        "auth_token",
        "token",
        "env",
        "environment",
        "password",
    ] {
        assert!(!ALLOWED_KEYS.contains(&forbidden), "{forbidden}");
        assert!(!ALL_KEYS.contains(&forbidden), "{forbidden}");
    }
    assert_eq!(ALLOWED_KEYS.len(), 44);
    // 闭集里没有重复键，否则计数就失去意义。
    let mut unique = ALLOWED_KEYS.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ALLOWED_KEYS.len());
}
