//! 维护确认、范围漂移、写者独占与首次初始化的目录前置。
use super::{
    super::{
        MaintenanceError,
        inputs::Inputs,
        options::{Command, Options, parse},
        workflow::execute,
    },
    sync::Sync,
    sync_rebind::{edit_approval, options as rebind_options},
};
use serde_json::Value;
use std::{ffi::OsString, fs};
use studiolo_vault_broker::{FsVaultBroker, MaintenanceActor};

fn options(s: &Sync, command: Command) -> Options {
    let mut options = rebind_options(s);
    options.command = command;
    options
}
fn run(
    s: &Sync,
    command: Command,
    confirm: impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    let options = options(s, command);
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
fn anchors_require_explicit_confirmation_preserve_content_and_do_not_initialize_sync_state() {
    let s = Sync::new(true);
    let vault = s.root().join("vault");
    let content = fs::read(vault.join("note.md")).unwrap();
    assert_eq!(
        run(&s, Command::SyncAnchors, |review, expected| {
            assert_eq!(expected, "sync-anchors");
            assert_eq!(review["missing_anchors"], serde_json::json!(["笔记"]));
            assert!(!vault.join("笔记").exists());
            Err(MaintenanceError::Confirmation)
        }),
        Err(MaintenanceError::Confirmation)
    );
    assert!(!vault.join("笔记").exists());
    let report = run(&s, Command::SyncAnchors, |_, _| Ok(())).unwrap();
    assert_eq!(report["created_anchors"], serde_json::json!(["笔记"]));
    assert!(vault.join("笔记").is_dir());
    assert_eq!(fs::read(vault.join("note.md")).unwrap(), content);
    assert_eq!(fs::read_dir(vault.join("笔记")).unwrap().count(), 0);
    for root in ["transport", "plane", "mirror", "mirror-view"] {
        assert_eq!(fs::read_dir(s.root().join(root)).unwrap().count(), 0);
    }
    let again = run(&s, Command::SyncAnchors, |_, _| Ok(())).unwrap();
    assert_eq!(again["created_anchors"], serde_json::json!([]));
    assert_eq!(again["committed_through"], report["committed_through"]);
}

#[test]
fn confirmation_time_configuration_drift_refuses_before_directory_creation() {
    for (command, scope_change) in [
        (Command::SyncAnchors, false),
        (Command::SyncAnchors, true),
        (Command::SyncInit, false),
        (Command::SyncInit, true),
    ] {
        let s = Sync::new(true);
        assert_eq!(
            run(&s, command, |_, _| {
                if scope_change {
                    let path = s.root().join("config/owner.json");
                    let mut value: Value =
                        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                    value["sync_scope"]["scope_epoch"] = 2.into();
                    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
                } else {
                    edit_approval(&s, |value| {
                        value["budget"]["deadline_seconds"] = 1800.into()
                    });
                }
                Ok(())
            }),
            Err(MaintenanceError::Configuration)
        );
        assert!(!s.root().join("vault/笔记").exists());
        assert_eq!(fs::read_dir(s.root().join("transport")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(s.root().join("plane")).unwrap().count(), 0);
    }
}

#[test]
fn missing_scope_and_live_owner_cannot_prepare_anchors() {
    let s = Sync::new(false);
    assert_eq!(
        s.run(Command::SyncAnchors, None),
        Err(MaintenanceError::Configuration)
    );
    let s = Sync::new(true);
    let _owner =
        FsVaultBroker::open_existing(&s.root().join("vault"), &s.root().join("data")).unwrap();
    assert_eq!(
        s.run(Command::SyncAnchors, None),
        Err(MaintenanceError::Broker)
    );
    assert!(!s.root().join("vault/笔记").exists());
}

#[test]
fn init_rejects_bad_anchor_before_any_private_store_and_cancellation_creates_nothing() {
    let s = Sync::new(true);
    fs::write(s.root().join("vault/笔记"), b"must survive").unwrap();
    assert_eq!(
        s.run(Command::SyncInit, None),
        Err(MaintenanceError::SyncAnchors)
    );
    assert_eq!(fs::read_dir(s.root().join("transport")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(s.root().join("plane")).unwrap().count(), 0);
    assert_eq!(
        fs::read(s.root().join("vault/笔记")).unwrap(),
        b"must survive"
    );
    let s = Sync::new(true);
    assert_eq!(
        run(&s, Command::SyncInit, |_, expected| {
            assert_eq!(expected, "sync-init");
            Err(MaintenanceError::Confirmation)
        }),
        Err(MaintenanceError::Confirmation)
    );
    assert!(!s.root().join("vault/笔记").exists());
    assert_eq!(fs::read_dir(s.root().join("transport")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(s.root().join("plane")).unwrap().count(), 0);
    let report = s.run(Command::SyncInit, None).unwrap();
    assert_eq!(report["created_anchors"], serde_json::json!(["笔记"]));
    assert!(s.root().join("vault/笔记").is_dir());
}

#[test]
fn anchors_cli_requires_approval_and_has_no_force_or_scope_override() {
    let args: Vec<OsString> = [
        "sync-anchors",
        "--config",
        "/private/config.json",
        "--vault",
        "/vault",
        "--assets",
        "/assets",
        "--writer-data",
        "/writer",
        "--sync-transport",
        "/private/approval.json",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    let parsed = parse(args.clone()).unwrap().unwrap();
    assert_eq!(parsed.command, Command::SyncAnchors);
    assert_eq!(parsed.command.name(), "sync-anchors");
    let mut missing = args.clone();
    missing.truncate(9);
    assert!(parse(missing).is_err());
    for flag in ["--force", "--yes", "--folder", "--request", "--scope"] {
        let mut bad = args.clone();
        bad[9] = flag.into();
        assert!(parse(bad).is_err());
    }
}
