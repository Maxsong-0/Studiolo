//! 维护能力只用于独立 CLI，具体系统调用保持封闭。

use crate::support::{rust_sources, source_violations, workspace_root};
use std::{fs, path::Path};

#[test]
fn maintenance_terminal_cannot_expand_to_other_files_or_commands() {
    let path = Path::new("crates/local-server/src/maintenance/terminal.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-local-server", path, &source).is_empty());
    for extra in [
        "fs::open(other, flags, mode);",
        "std::fs::write(path, bytes);",
        "File::create(path);",
        "other.write_all(bytes);",
        "Command::new(command);",
    ] {
        assert!(
            !source_violations("studiolo-local-server", path, &format!("{source}\n{extra}"))
                .is_empty(),
            "{extra}"
        );
    }
    let changed = source.replace("/dev/tty", "/tmp/other");
    assert!(!source_violations("studiolo-local-server", path, &changed).is_empty());
}

#[test]
fn maintenance_is_not_an_http_factory_or_an_alternate_writer() {
    let root = workspace_root().join("crates/local-server/src");
    let inputs = fs::read_to_string(root.join("maintenance/inputs.rs")).unwrap();
    let compact = inputs
        .split_whitespace()
        .collect::<String>()
        .replace(",)", ")");
    assert_eq!(compact.matches("FsVaultBroker::open_existing(").count(), 1);
    assert!(compact.contains(
        "FsVaultBroker::open_existing(roots.vault_directory(),roots.writer_data_directory())"
    ));
    assert!(compact.contains(
        "broker.validate_root_identities(roots.vault_identity(),roots.writer_data_identity())"
    ));
    assert!(compact.contains("Config::load(&options.config)"));
    for path in rust_sources(&root) {
        let source = fs::read_to_string(&path).unwrap();
        if path.starts_with(root.join("maintenance"))
            || path == root.join("maintenance.rs")
            || path == root.join("bin/studiolo-maintain.rs")
        {
            for forbidden in [
                "VaultWriter",
                "Mutation::",
                "Command::new(",
                "Connection::open",
            ] {
                assert!(!source.contains(forbidden), "{path:?}: {forbidden}");
            }
        } else {
            for forbidden in [
                "maintenance::run(",
                ".prepare_maintenance(",
                ".commit_maintenance(",
                ".cancel_maintenance(",
            ] {
                assert!(!source.contains(forbidden), "{path:?}: {forbidden}");
            }
        }
    }
}
