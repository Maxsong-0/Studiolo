use crate::support::{
    AuthorityViolation, crate_directories, package_name, rust_sources, source_violations,
    workspace_root,
};
use std::{fs, path::Path};

#[test]
fn authority_scanner_detects_sneaky_source_fixture() {
    let fixture = r#"
        use cap_std::ambient_authority;
        let root = Dir::open_ambient_dir(candidate, ambient_authority());
        std::fs::write(root.join(".workbench/vault.json"), bytes)?;
    "#;

    let violations = source_violations(
        "studiolo-gateway",
        Path::new("crates/gateway/src/sneaky.rs"),
        fixture,
    );

    assert!(violations.contains(&AuthorityViolation::CanonicalPath));
    assert!(violations.contains(&AuthorityViolation::AmbientRoot));
    assert!(violations.contains(&AuthorityViolation::GenericWrite));

    let legitimate_runner_temp = r#"
        struct StubRun { temp_dir: TempDir }
        let result_path = temp_dir.path().join("result.md");
        let mut file = File::create(&result_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    "#;
    assert!(
        source_violations(
            "studiolo-runner-adapter",
            Path::new("crates/runner-adapter/src/lib.rs"),
            legitimate_runner_temp,
        )
        .is_empty()
    );

    let sneaky_runner_write = r#"
        struct StubRun { temp_dir: TempDir }
        let mut file = File::create("/srv/canonical-vault/stolen.md")?;
        file.write_all(bytes)?;
    "#;
    assert!(
        source_violations(
            "studiolo-runner-adapter",
            Path::new("crates/runner-adapter/src/sneaky.rs"),
            sneaky_runner_write,
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
    assert!(
        source_violations(
            "studiolo-task-engine",
            Path::new("src/alias.rs"),
            "use std::{fs::{write as save}}; save(path, bytes)?;"
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
}

#[test]
fn only_vault_broker_source_owns_canonical_vault_authority() {
    let root = workspace_root();
    let mut findings = Vec::new();
    let mut broker_source = String::new();

    for crate_directory in crate_directories(&root) {
        let manifest = fs::read_to_string(crate_directory.join("Cargo.toml"))
            .expect("read workspace crate manifest");
        let crate_name = package_name(&manifest);
        for source_path in rust_sources(&crate_directory.join("src")) {
            let source = fs::read_to_string(&source_path).expect("read workspace source");
            if crate_name == "studiolo-vault-broker" {
                broker_source.push_str(&source);
            }
            for violation in source_violations(&crate_name, &source_path, &source) {
                findings.push(format!(
                    "{}: {violation:?}",
                    source_path
                        .strip_prefix(&root)
                        .expect("source is inside workspace")
                        .display()
                ));
            }
        }
    }

    assert!(
        findings.is_empty(),
        "canonical Vault authority escaped vault-broker:\n{}",
        findings.join("\n")
    );
    // `Broker` 现在从固定的操作系统锚点出发，以 `nofollow` 方式逐级遍历目录；
    // 归属规则覆盖它的全部源文件，不再依赖旧的辅助函数。
    assert!(!broker_source.is_empty());
    assert!(broker_source.contains(".workbench/vault.json"));
    assert!(broker_source.contains("sources/attempts/"));
}

#[test]
fn fixed_agent_database_creation_pin_does_not_allow_another_file_or_write() {
    let path = Path::new("crates/supply/src/storage.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-supply", path, &source).is_empty());
    for extra in [
        "std::fs::File::create(other)?;",
        "pin.write_all(body)?;",
        "std::fs::File::open(other)?;",
    ] {
        let injected = format!("{source}\n{extra}");
        assert!(
            source_violations("studiolo-supply", path, &injected)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    assert!(
        source_violations(
            "studiolo-supply",
            Path::new("crates/supply/src/extra.rs"),
            "struct Unexpected { created_pin: Option<std::fs::File>, }",
        )
        .contains(&AuthorityViolation::GenericWrite)
    );
}

#[path = "m5_m6_authority.rs"]
mod m5_m6_authority;

#[test]
fn vault_initializer_source_can_only_inspect_approved_roots_and_read_config() {
    let path = Path::new("crates/vault-admin/src/source.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-vault-admin", path, &source).is_empty());
    for changed in [
        source.replacen("OFlags::RDONLY", "OFlags::RDWR", 1),
        source.replacen("OFlags::NOFOLLOW |", "", 1),
        format!("{source}\nfs::write(path, bytes)?;"),
        format!("{source}\nFile::create(path)?;"),
        format!("{source}\nopen(other, OFlags::RDWR, Mode::empty())?;"),
    ] {
        assert!(
            source_violations("studiolo-vault-admin", path, &changed)
                .contains(&AuthorityViolation::GenericWrite)
        );
    }
    assert!(
        source_violations("studiolo-vault-admin", Path::new("src/other.rs"), &source)
            .contains(&AuthorityViolation::GenericWrite)
    );
}

#[test]
fn offline_restore_descriptor_pins_remain_read_only_and_layout_bound() {
    let path = Path::new("crates/auth-admin/src/restore_guard.rs");
    let source = fs::read_to_string(workspace_root().join(path)).unwrap();
    assert!(source_violations("studiolo-auth-admin", path, &source).is_empty());
    for (before, after) in [
        ("OFlags::RDONLY", "OFlags::RDWR"),
        (".union(OFlags::NOFOLLOW)", ""),
        ("/var/lib/studiolo-dr-tests", "/srv/canonical-vault"),
        ("layout(root, config)?;", ""),
        ("real != 0", "false"),
        ("!separate", "false"),
    ] {
        assert!(source.contains(before));
        let changed = source.replacen(before, after, 1);
        assert!(
            source_violations("studiolo-auth-admin", path, &changed)
                .contains(&AuthorityViolation::GenericWrite),
            "{before}"
        );
    }
    for extra in [
        "File::create(other)?;",
        "File::open(other)?;",
        "file.write_all(body)?;",
        "openat(&pinned.file, other, FLAGS, Mode::empty())?;",
    ] {
        let changed = format!("{source}\n{extra}");
        assert!(
            source_violations("studiolo-auth-admin", path, &changed)
                .contains(&AuthorityViolation::GenericWrite),
            "{extra}"
        );
    }
    assert!(
        source_violations("studiolo-auth-admin", Path::new("src/other.rs"), &source)
            .contains(&AuthorityViolation::GenericWrite)
    );
}
