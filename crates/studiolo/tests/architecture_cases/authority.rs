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
    // Broker now opens a nofollow directory walk from a fixed OS anchor. The
    // ownership rule covers all its sources without requiring the old helper.
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
