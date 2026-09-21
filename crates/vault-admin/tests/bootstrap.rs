//! 使用真实私有根、真实 Broker 与 SQLite；不访问用户资料或模型。

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_vault_admin::{Error, Options, initialize, parse, plan};
use studiolo_vault_broker::{FsVaultBroker, VaultReader};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    options: Options,
}

fn directory(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn file(path: &Path, content: &[u8]) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for name in [
            "config", "auth", "audit", "vault", "data", "assets", "source",
        ] {
            directory(&root.join(name));
        }
        let config = root.join("config/owner.json");
        file(&config, &serde_json::to_vec(&serde_json::json!({
            "version":1,"auth_directory":root.join("auth"),"audit_directory":root.join("audit"),
            "owner":{"issuer":"https://fixture.cloudflareaccess.com","audience":"fixture-audience",
                "subject":"fictional-owner","email":"owner@example.invalid"},
            "idle_seconds":1800,"absolute_seconds":43200,"seed_ref":"fixture-only"
        })).unwrap());
        directory(&root.join("source/项目"));
        directory(&root.join("source/项目/Studiolo"));
        directory(&root.join("source/其他"));
        file(
            &root.join("source/项目/Studiolo/a.md"),
            "# 正式首建\n".as_bytes(),
        );
        file(&root.join("source/项目/Studiolo/b.md"), b"second\n");
        file(&root.join("source/其他/excluded.md"), b"excluded content\n");
        drop(SqliteAuditLedger::open(&root.join("audit")).unwrap());
        let options = Options {
            config,
            vault: root.join("vault"),
            writer_data: root.join("data"),
            assets: root.join("assets"),
            source_snapshot: root.join("source"),
            allow: vec![VaultPath::parse("项目/Studiolo").unwrap()],
        };
        Self {
            _temp: temp,
            root,
            options,
        }
    }

    fn empty_targets(&self) {
        assert_eq!(fs::read_dir(&self.options.vault).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&self.options.writer_data).unwrap().count(), 0);
    }

    fn audit_count(&self) -> usize {
        SqliteAuditLedger::open_existing(&self.root.join("audit"))
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap()
            .len()
    }
}

fn inventory(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let bytes = if metadata.is_dir() {
                Vec::new()
            } else {
                fs::read(&path).unwrap()
            };
            result.insert(
                path.strip_prefix(root).unwrap().into(),
                (metadata.permissions().mode(), bytes),
            );
            if metadata.is_dir() {
                visit(root, &path, result);
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

#[test]
fn plan_has_no_persistent_writes_and_only_outputs_summary() {
    let f = Fixture::new();
    let before = inventory(&f.root);
    let first = plan(&f.options).unwrap();
    assert_eq!(first, plan(&f.options).unwrap());
    assert_eq!(first.files, 2);
    assert_eq!(first.bytes, "# 正式首建\n".len() as u64 + 7);
    assert_eq!(first.plan_digest.len(), 64);
    let output = serde_json::to_string(&first).unwrap();
    assert!(!output.contains("正式首建"));
    assert!(!output.contains("a.md"));
    assert!(!output.contains(f.root.to_str().unwrap()));
    assert_eq!(before, inventory(&f.root));
    f.empty_targets();
    assert_eq!(f.audit_count(), 0);
}

#[test]
fn source_changes_or_wrong_digest_reject_before_initialization_and_audit() {
    for change_source in [false, true] {
        let f = Fixture::new();
        let approved = plan(&f.options).unwrap();
        let expected = if change_source {
            file(
                &f.options.source_snapshot.join("项目/Studiolo/a.md"),
                b"changed\n",
            );
            approved.plan_digest
        } else {
            "00".repeat(32)
        };
        assert_eq!(initialize(&f.options, &expected), Err(Error::PlanMismatch));
        f.empty_targets();
        assert_eq!(f.audit_count(), 0);
    }
}

#[test]
fn plan_binds_target_identity_config_revision_and_scope() {
    for changed in ["target", "config", "scope"] {
        let mut f = Fixture::new();
        let approved = plan(&f.options).unwrap().plan_digest;
        match changed {
            "target" => {
                fs::rename(&f.options.vault, f.root.join("previous-vault")).unwrap();
                directory(&f.options.vault);
            }
            "config" => {
                let mut config: serde_json::Value =
                    serde_json::from_slice(&fs::read(&f.options.config).unwrap()).unwrap();
                config["idle_seconds"] = 900.into();
                file(&f.options.config, &serde_json::to_vec(&config).unwrap());
            }
            _ => f.options.allow = vec![VaultPath::parse("项目/Studiolo/a.md").unwrap()],
        }
        assert_eq!(
            initialize(&f.options, &approved),
            Err(Error::PlanMismatch),
            "{changed}"
        );
        f.empty_targets();
    }
}

#[test]
fn overlapping_source_and_nonempty_targets_are_rejected_without_writes() {
    for target in [
        "source",
        "nested-source",
        "auth",
        "assets",
        "config",
        "data",
        "vault",
    ] {
        let mut f = Fixture::new();
        match target {
            "source" => f.options.vault = f.options.source_snapshot.clone(),
            "nested-source" => {
                directory(&f.options.source_snapshot.join("target"));
                f.options.vault = f.options.source_snapshot.join("target");
            }
            "auth" => f.options.source_snapshot = f.root.join("auth"),
            "assets" => f.options.source_snapshot = f.options.assets.clone(),
            "config" => f.options.source_snapshot = f.root.join("config"),
            "data" => file(&f.options.writer_data.join("existing"), b"preserve"),
            _ => file(&f.options.vault.join("existing"), b"preserve"),
        }
        let before = inventory(&f.root);
        assert!(plan(&f.options).is_err(), "{target}");
        assert_eq!(before, inventory(&f.root), "{target}");
    }
}

#[test]
fn links_diagnostics_selected_attachments_and_invalid_content_fail_closed() {
    for damage in [
        "root-link",
        "file-link",
        "attachment",
        "permissions",
        "hidden",
        "utf8",
    ] {
        let mut f = Fixture::new();
        match damage {
            "root-link" => {
                let alias = f.root.join("source-link");
                symlink(&f.options.source_snapshot, &alias).unwrap();
                f.options.source_snapshot = alias;
            }
            "file-link" => symlink(
                "a.md",
                f.options.source_snapshot.join("项目/Studiolo/link.md"),
            )
            .unwrap(),
            "attachment" => file(
                &f.options.source_snapshot.join("项目/Studiolo/a.png"),
                b"attachment",
            ),
            "permissions" => fs::set_permissions(
                f.options.source_snapshot.join("项目/Studiolo/a.md"),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap(),
            "hidden" => file(&f.options.source_snapshot.join(".hidden"), b"hidden"),
            _ => file(
                &f.options.source_snapshot.join("项目/Studiolo/a.md"),
                &[0xff, 0xfe],
            ),
        }
        assert!(plan(&f.options).is_err(), "{damage}");
        f.empty_targets();
        assert_eq!(f.audit_count(), 0);
    }
}

#[test]
fn real_import_reopens_and_second_initialization_cannot_touch_existing_data() {
    let f = Fixture::new();
    let approved = plan(&f.options).unwrap();
    let result = initialize(&f.options, &approved.plan_digest).unwrap();
    assert_eq!(result.files, 2);
    let broker = FsVaultBroker::open_existing(&f.options.vault, &f.options.writer_data).unwrap();
    assert_eq!(result.vault_uuid, broker.vault_uuid().to_string());
    assert_eq!(
        broker
            .read(&VaultPath::parse("项目/Studiolo/a.md").unwrap())
            .unwrap()
            .content,
        "# 正式首建\n".as_bytes()
    );
    assert!(
        broker
            .read(&VaultPath::parse("其他/excluded.md").unwrap())
            .is_err()
    );
    drop(broker);
    let audit = SqliteAuditLedger::open_existing(&f.root.join("audit")).unwrap();
    let records = audit.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    assert_eq!(
        records[1].event().payload().as_value()["status"],
        "completed"
    );
    assert_eq!(
        records[0].event().payload().as_value()["actor_kind"],
        "local_os_bootstrap"
    );
    drop(audit);
    let before = inventory(&f.root);
    assert!(initialize(&f.options, &approved.plan_digest).is_err());
    assert_eq!(before, inventory(&f.root));
}

#[test]
fn missing_audit_is_not_created_and_prevents_bootstrap() {
    let f = Fixture::new();
    let approved = plan(&f.options).unwrap();
    fs::remove_file(f.root.join("audit/audit.sqlite3")).unwrap();
    assert_eq!(
        initialize(&f.options, &approved.plan_digest),
        Err(Error::Audit)
    );
    assert_eq!(fs::read_dir(f.root.join("audit")).unwrap().count(), 0);
    f.empty_targets();
}

#[test]
fn parser_requires_closed_command_explicit_fields_and_digest() {
    let args = [
        "plan",
        "--config",
        "/config/owner.json",
        "--vault",
        "/vault",
        "--writer-data",
        "/data",
        "--assets",
        "/assets",
        "--source-snapshot",
        "/source",
        "--allow",
        "项目/Studiolo",
    ];
    assert!(
        parse(args.iter().map(Into::into).collect())
            .unwrap()
            .is_some()
    );
    for extra in [
        ["--expected-plan", "bad"],
        ["--config", "/again"],
        ["--unknown", "/value"],
    ] {
        assert!(parse(args.iter().chain(extra.iter()).map(Into::into).collect()).is_err());
    }
    let mut initialize_args = args.to_vec();
    initialize_args[0] = "initialize";
    assert!(parse(initialize_args.iter().map(Into::into).collect()).is_err());
    assert!(parse(vec!["--help".into()]).unwrap().is_none());
}

#[test]
fn cli_overrides_inherited_umask_and_keeps_every_created_node_private() {
    fn invoke(f: &Fixture, operation: &str, expected: Option<&str>) -> std::process::Output {
        // 只在子进程改 umask；所有参数独立传入，不做 shell 字符串拼接或 unsafe pre_exec。
        let mut command = std::process::Command::new("/bin/sh");
        command.args([
            "-c",
            "umask 022; exec \"$@\"",
            "studiolo-vault-admin-permissions",
        ]);
        command.arg(env!("CARGO_BIN_EXE_studiolo-vault-admin"));
        command.arg(operation);
        for (name, path) in [
            ("--config", &f.options.config),
            ("--vault", &f.options.vault),
            ("--writer-data", &f.options.writer_data),
            ("--assets", &f.options.assets),
            ("--source-snapshot", &f.options.source_snapshot),
        ] {
            command.arg(name).arg(path);
        }
        command.args(["--allow", "项目/Studiolo"]);
        if let Some(expected) = expected {
            command.args(["--expected-plan", expected]);
        }
        command.output().unwrap()
    }

    let f = Fixture::new();
    let before = inventory(&f.root);
    let planned = invoke(&f, "plan", None);
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    assert_eq!(before, inventory(&f.root));
    let plan: serde_json::Value = serde_json::from_slice(&planned.stdout).unwrap();
    let initialized = invoke(&f, "initialize", plan["plan_digest"].as_str());
    assert!(
        initialized.status.success(),
        "{}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&initialized.stdout).unwrap();
    assert_eq!(result["files"], 2);
    for root in [&f.options.vault, &f.options.writer_data] {
        let created = inventory(root);
        assert!(!created.is_empty());
        for (path, (mode, _)) in created {
            let expected = if root.join(&path).is_dir() {
                0o700
            } else {
                0o600
            };
            assert_eq!(mode & 0o7777, expected, "{}", path.display());
        }
    }
    let broker = FsVaultBroker::open_existing(&f.options.vault, &f.options.writer_data).unwrap();
    assert_eq!(
        broker
            .read(&VaultPath::parse("项目/Studiolo/a.md").unwrap())
            .unwrap()
            .content,
        "# 正式首建\n".as_bytes()
    );
}
