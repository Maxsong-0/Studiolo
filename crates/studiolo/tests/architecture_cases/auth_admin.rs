use crate::support::{AuthorityViolation, source_violations, workspace_root};
use std::path::Path;

#[test]
fn shared_configuration_adapter_is_read_only_and_local() {
    let path = Path::new("crates/host-config/src/config_file.rs");
    let source = std::fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-host-config", path, &source).is_empty());
    for changed in [
        source.replace("OFlags::RDONLY", "OFlags::RDWR"),
        source.replace("OFlags::NOFOLLOW", "OFlags::CREATE"),
        format!("{source}\nfs::write(path, bytes);"),
        format!("{source}\nlet other = open(path, flags, mode);"),
        format!("{source}\nCommand::new(\"sh\");"),
    ] {
        assert!(
            source_violations("studiolo-host-config", path, &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    for (name, wrong_path) in [
        ("studiolo-auth", path),
        ("studiolo-auth-admin", path),
        (
            "studiolo-host-config",
            Path::new("crates/host-config/src/lib.rs"),
        ),
    ] {
        assert!(
            source_violations(name, wrong_path, &source)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
}

#[test]
fn admin_production_build_is_explicit_in_verification() {
    let root = workspace_root();
    for path in ["scripts/test-linux.sh", ".github/workflows/ci.yml"] {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        assert!(source.contains("-p studiolo-auth-admin --features production"));
    }
    let manifest = std::fs::read_to_string(root.join("crates/auth-admin/Cargo.toml")).unwrap();
    assert!(manifest.contains("production = [\"studiolo-auth/production\"]"));
    assert!(!manifest.contains("studiolo-auth/test-support"));
}

#[test]
fn admin_terminal_adapter_cannot_open_or_write_an_arbitrary_file() {
    let path = Path::new("crates/auth-admin/src/terminal.rs");
    let source = std::fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-auth-admin", path, &source).is_empty());
    for changed in [
        source.replace("/dev/tty", "/private/other"),
        source.replace("OFlags::NOCTTY", "OFlags::CREATE"),
        format!("{source}\nFile::open(path);"),
        format!("{source}\nfs::open(path, flags, mode);"),
        format!("{source}\n(&hidden.file).write_all(secret);"),
        format!("{source}\nCommand::new(\"sh\");"),
    ] {
        assert!(
            source_violations("studiolo-auth-admin", path, &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    assert!(
        source_violations(
            "studiolo-auth-admin",
            Path::new("crates/auth-admin/src/workflow.rs"),
            &source
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
}
