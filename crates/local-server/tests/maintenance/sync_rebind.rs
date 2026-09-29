//! 真实维护入口：确认边界、四根路径改名、scope 漂移和不触碰 stage 孤儿。

use super::{
    super::{
        MaintenanceError,
        inputs::Inputs,
        options::{Command, Options},
        workflow::execute,
    },
    sync::{Sync, private_dir},
};
use serde_json::{Value, json};
use std::fs;
use studiolo_sync_transport::owned_run::{TransportError, TransportState, state};
use studiolo_vault_broker::MaintenanceActor;

pub(super) fn initialized() -> Sync {
    let s = Sync::new(true);
    s.run(Command::SyncInit, None).unwrap();
    s
}

pub(super) fn options(s: &Sync) -> Options {
    Options {
        command: Command::SyncRebind,
        config: s.root().join("config/owner.json"),
        vault: s.root().join("vault"),
        assets: s.root().join("assets"),
        writer_data: s.root().join("data"),
        request: None,
        maintenance: None,
        sync_transport: Some(s.root().join("config/sync-approval.json")),
        folder: None,
    }
}

pub(super) fn edit_approval(s: &Sync, edit: impl FnOnce(&mut Value)) {
    let path = s.root().join("config/sync-approval.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    edit(&mut document);
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
}

pub(super) fn state_bytes(s: &Sync) -> Vec<u8> {
    fs::read(s.root().join("transport/state.json")).unwrap()
}

fn run_with(
    s: &Sync,
    confirm: impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    let options = options(s);
    let mut inputs = Inputs::open(&options)?;
    execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        confirm,
        || Ok(()),
    )
}

#[test]
fn runtime_backend_budget_and_all_four_path_fields_can_be_explicitly_rebound() {
    for field in [
        "runtime",
        "backend",
        "budget",
        "private_root",
        "mirror_root",
        "owner_view_root",
        "plane_root",
    ] {
        let s = initialized();
        let old = state::decode(&state_bytes(&s)).unwrap();
        let mut state_path = s.root().join("transport/state.json");
        edit_approval(&s, |document| match field {
            "runtime" => document[field]["closure_digest"] = json!("2".repeat(64)),
            "backend" => document[field]["launcher_path"] = json!("/opt/studiolo/new-launcher"),
            "budget" => document[field]["deadline_seconds"] = json!(1800),
            _ => {
                let from = document[field].as_str().unwrap();
                let to = s.root().join(format!("renamed-{field}"));
                fs::rename(from, &to).unwrap();
                if field == "private_root" {
                    state_path = to.join("state.json");
                }
                document[field] = json!(to);
            }
        });
        let result = run_with(&s, |review, expected| {
            assert_eq!(expected, "sync-rebind");
            assert_eq!(review["binding"]["previous_generation"], old.generation);
            assert_eq!(review["binding"]["next_generation"], old.generation + 1);
            assert!(!review.to_string().contains(s.root().to_str().unwrap()));
            Ok(())
        })
        .unwrap();
        assert_eq!(result["command"], "sync-rebind");
        let new: TransportState = state::decode(&fs::read(state_path).unwrap()).unwrap();
        assert_eq!(new.installation_id, old.installation_id);
        assert_eq!(new.next_sequence, old.next_sequence);
        assert_eq!(new.generation, old.generation + 1);
        assert_ne!(
            new.binding.approved_config_digest,
            old.binding.approved_config_digest
        );
        assert!(new.last_terminal.is_none());
    }
}

#[test]
fn refused_confirmation_and_confirmation_time_approval_drift_never_publish() {
    let s = initialized();
    edit_approval(&s, |doc| doc["budget"]["deadline_seconds"] = json!(1200));
    let old = state_bytes(&s);
    assert_eq!(
        run_with(&s, |_, _| Err(MaintenanceError::Confirmation)),
        Err(MaintenanceError::Confirmation)
    );
    assert_eq!(state_bytes(&s), old);
    assert_eq!(
        run_with(&s, |_, _| {
            edit_approval(&s, |doc| doc["budget"]["deadline_seconds"] = json!(1500));
            Ok(())
        }),
        Err(MaintenanceError::Configuration)
    );
    assert_eq!(state_bytes(&s), old);
}

#[test]
fn scope_changes_are_refused_without_touching_the_transport_slot() {
    let s = initialized();
    let old = state_bytes(&s);
    let config = s.root().join("config/owner.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    document["sync_scope"]["scope_epoch"] = json!(2);
    fs::write(config, serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(
        s.run(Command::SyncRebind, None),
        Err(MaintenanceError::SyncRebind(
            TransportError::BindingMismatch
        ))
    );
    assert_eq!(state_bytes(&s), old);
}

#[test]
fn copied_private_or_replaced_mirror_inode_is_refused() {
    for private in [true, false] {
        let s = initialized();
        let old = state_bytes(&s);
        let replacement = s.root().join("replacement");
        private_dir(&replacement);
        if private {
            for file in ["state.json", "transport.lock"] {
                fs::copy(
                    s.root().join("transport").join(file),
                    replacement.join(file),
                )
                .unwrap();
            }
        }
        edit_approval(&s, |doc| {
            doc[if private {
                "private_root"
            } else {
                "owner_view_root"
            }] = json!(replacement);
        });
        assert_eq!(
            s.run(Command::SyncRebind, None),
            Err(MaintenanceError::SyncRebind(TransportError::RootsChanged))
        );
        assert_eq!(state_bytes(&s), old);
    }
}

#[test]
fn rebind_never_collects_stage_orphans_and_requires_all_existing_stores() {
    let s = initialized();
    let orphan = s.root().join("plane/stage/objects").join("ab".repeat(32));
    fs::write(&orphan, "保留孤儿").unwrap();
    s.run(Command::SyncRebind, None).unwrap();
    assert_eq!(fs::read_to_string(&orphan).unwrap(), "保留孤儿");
    for relative in [
        "plane/plane.lock",
        "plane/plane-state.json",
        "plane/stage/index.json",
    ] {
        let s = initialized();
        let old = state_bytes(&s);
        fs::remove_file(s.root().join(relative)).unwrap();
        assert_eq!(
            s.run(Command::SyncRebind, None),
            Err(MaintenanceError::SyncStorage)
        );
        assert!(!s.root().join(relative).exists());
        assert_eq!(state_bytes(&s), old);
    }
}
