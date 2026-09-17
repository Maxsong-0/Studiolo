//! 真实 CLI + PTY + symlink：维护自身收口不读取原正文或链接目标。

use super::{Fixture, binary, process_support};
use serde_json::Value;
use std::{fs, os::unix::fs::symlink};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    FsVaultBroker, MaintenanceActor, MaintenanceDecision, Mutation, MutationActor, MutationKind,
    MutationMetadata,
};

fn unexecuted(fixture: &Fixture) -> String {
    let mut broker =
        FsVaultBroker::open_existing(&fixture.root.join("vault"), &fixture.root.join("writer"))
            .unwrap();
    let path = VaultPath::parse("note.md").unwrap();
    let actor = MutationActor::Human {
        issuer: "https://original.example.com".into(),
        subject: "original-human".into(),
    };
    let source = broker
        .mutate_recorded(
            Mutation::create(path.clone(), b"ORIGINAL_PRIVATE_BODY".to_vec(), "first"),
            MutationMetadata::new(actor.clone(), MutationKind::Replace).unwrap(),
        )
        .unwrap()
        .revision;
    let current = broker
        .mutate_recorded(
            Mutation::replace(
                path.clone(),
                b"CURRENT_PRIVATE_BODY".to_vec(),
                source,
                "second",
            ),
            MutationMetadata::new(actor.clone(), MutationKind::Replace).unwrap(),
        )
        .unwrap()
        .revision;
    drop(
        broker
            .prepare_recorded_restore(
                Mutation::replace(
                    path,
                    b"ORIGINAL_PRIVATE_BODY".to_vec(),
                    current,
                    "LOST_PRIVATE_KEY",
                ),
                MutationMetadata::new(
                    actor,
                    MutationKind::Restore {
                        source_revision: source,
                    },
                )
                .unwrap(),
            )
            .unwrap(),
    );
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::CancelRestoreIntent,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    ticket.intent().event_id.clone()
}

fn report(output: &str, action: &str) -> Value {
    for forbidden in [
        "ORIGINAL_PRIVATE_BODY",
        "CURRENT_PRIVATE_BODY",
        "OUTSIDE_PRIVATE_BODY",
        "LOST_PRIVATE_KEY",
    ] {
        assert!(!output.contains(forbidden), "CLI不应输出正文或原请求密钥");
    }
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            serde_json::from_str::<Value>(line.strip_prefix("> ").unwrap_or(line)).ok()
        })
        .find(|value| value["command"] == action)
        .unwrap_or_else(|| panic!("缺少动作后的完整JSON报告：{output}"))
}

fn unchanged(fixture: &Fixture) {
    assert!(
        fs::symlink_metadata(fixture.root.join("vault/note.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read(fixture.root.join("outside.md")).unwrap(),
        b"OUTSIDE_PRIVATE_BODY"
    );
}

#[test]
fn real_pty_can_inspect_flush_and_cancel_maintenance_after_target_becomes_symlink() {
    let fixture = Fixture::new(true);
    let id = unexecuted(&fixture);
    fs::write(fixture.root.join("outside.md"), b"OUTSIDE_PRIVATE_BODY").unwrap();
    fs::remove_file(fixture.root.join("vault/note.md")).unwrap();
    symlink(
        fixture.root.join("outside.md"),
        fixture.root.join("vault/note.md"),
    )
    .unwrap();

    let (status, output) = process_support::output(
        process_support::script(binary(), &fixture.args("inspect")),
        None,
    );
    assert!(status.success(), "{output}");
    let inspected = report(&output, "inspect");
    assert_eq!(inspected["status"]["pending"]["maintenance_id"], id);
    assert_eq!(inspected["status"]["pending"]["execution_started"], false);
    unchanged(&fixture);

    let (status, output) = process_support::output(
        process_support::script(binary(), &fixture.args("audit-flush")),
        Some(b"audit-flush\n"),
    );
    assert!(status.success(), "{output}");
    let flushed = report(&output, "audit-flush");
    assert_eq!(flushed["status"]["pending"]["intent_confirmed"], true);
    assert_eq!(flushed["status"]["pending"]["execution_started"], false);
    assert_eq!(flushed["status"]["targets_status"], "not_observed");
    assert!(flushed["status"]["targets"].is_null());
    assert_eq!(flushed["content_audit_pending"], 0);
    unchanged(&fixture);

    let mut args = fixture.args("cancel-maintenance");
    args.extend(["--maintenance".into(), id.clone().into()]);
    let typed = format!("cancel-maintenance {id}\n");
    let (status, output) = process_support::output(
        process_support::script(binary(), &args),
        Some(typed.as_bytes()),
    );
    assert!(status.success(), "{output}");
    let cancelled = report(&output, "cancel-maintenance");
    assert_eq!(cancelled["receipt"]["event"]["outcome"], "cancelled");
    assert!(cancelled["receipt"]["event"]["resolution"].is_null());
    assert!(cancelled["status"]["pending"].is_null());
    assert_eq!(cancelled["status"]["targets_status"], "not_observed");
    assert!(cancelled["status"]["targets"].is_null());
    unchanged(&fixture);
    let broker =
        FsVaultBroker::open_existing(&fixture.root.join("vault"), &fixture.root.join("writer"))
            .unwrap();
    assert_eq!(broker.pending_recorded_restores(64).unwrap().len(), 1);
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    assert!(broker.maintenance_pending().unwrap().is_none());
}
