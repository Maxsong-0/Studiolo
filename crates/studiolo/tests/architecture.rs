//! Executable M0 architecture boundaries for canonical Vault authority.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthorityViolation {
    CanonicalPath,
    AmbientRoot,
    GenericWrite,
}

fn source_violations(
    crate_name: &str,
    _source_path: &Path,
    source: &str,
) -> Vec<AuthorityViolation> {
    if crate_name == "studiolo-vault-broker" {
        return Vec::new();
    }

    let mut violations = Vec::new();
    if [".workbench/vault.json", "sources/attempts/"]
        .iter()
        .any(|pattern| source.contains(pattern))
    {
        violations.push(AuthorityViolation::CanonicalPath);
    }
    if [
        "cap_std::ambient_authority",
        "ambient_authority()",
        "open_ambient_dir",
    ]
    .iter()
    .any(|pattern| source.contains(pattern))
    {
        violations.push(AuthorityViolation::AmbientRoot);
    }
    let generic_write_patterns = [
        "std::fs::write(",
        "fs::write(",
        "File::create(",
        "OpenOptions::new(",
        ".write_all(",
        ".sync_all(",
        ".create_dir(",
        ".create_dir_all(",
        ".rename(",
        ".remove_file(",
        ".remove_dir(",
        "Command::new(",
    ];
    let has_generic_write = generic_write_patterns
        .iter()
        .any(|pattern| source.contains(pattern));
    let runner_writes_only_its_temp_result = if crate_name == "studiolo-runner-adapter" {
        let mut remainder = source.to_owned();
        for allowed in [
            "File::create(&result_path)",
            "file.write_all(",
            "file.sync_all(",
        ] {
            remainder = remainder.replace(allowed, "");
        }
        source.contains("temp_dir.path().join(\"result.md\")")
            && !generic_write_patterns
                .iter()
                .any(|pattern| remainder.contains(pattern))
    } else {
        false
    };
    let imports_filesystem_authority = if crate_name == "studiolo-runner-adapter" {
        source.replace("fs::File,", "").contains("fs::")
    } else {
        source.contains("fs::")
    };
    if (has_generic_write && !runner_writes_only_its_temp_result) || imports_filesystem_authority {
        violations.push(AuthorityViolation::GenericWrite);
    }
    violations
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("studiolo crate lives at <workspace>/crates/studiolo")
        .to_path_buf()
}

fn crate_directories(root: &Path) -> Vec<PathBuf> {
    let mut directories = fs::read_dir(root.join("crates"))
        .expect("read workspace crates")
        .map(|entry| entry.expect("read crate directory entry").path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    fn visit(directory: &Path, output: &mut Vec<PathBuf>) {
        let mut entries = fs::read_dir(directory)
            .expect("read source directory")
            .map(|entry| entry.expect("read source entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let file_type = entry.file_type().expect("read source file type");
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                visit(&entry.path(), output);
            } else if entry.path().extension().is_some_and(|value| value == "rs") {
                output.push(entry.path());
            }
        }
    }

    let mut sources = Vec::new();
    if root.is_dir() {
        visit(root, &mut sources);
    }
    sources
}

fn package_name(manifest: &str) -> String {
    let mut in_package = false;
    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "name"
        {
            return value.trim().trim_matches('"').to_owned();
        }
    }
    panic!("manifest has no [package] name")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManifestInfo {
    name: String,
    dependencies: BTreeSet<String>,
}

fn parse_manifest(manifest: &str) -> ManifestInfo {
    let mut name = None;
    let mut dependencies = BTreeSet::new();
    let mut section = String::new();
    let mut table_dependency = None::<String>;

    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(value) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            section = value.trim().to_owned();
            table_dependency = dependency_table_name(&section).map(str::to_owned);
            if let Some(dependency) = &table_dependency {
                dependencies.insert(dependency.clone());
            }
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().trim_matches(['"', '\'']);
        let value = raw_value.trim();
        if section == "package" && key == "name" {
            name = Some(value.trim_matches('"').to_owned());
            continue;
        }
        if is_dependency_section(&section) {
            assert!(
                !key.contains('.') || key.ends_with(".workspace"),
                "architecture dependency scanner requires explicit dependency tables for dotted declarations"
            );
            assert!(
                !value.starts_with('{') || value.ends_with('}'),
                "architecture dependency scanner requires inline tables on one line"
            );
            dependencies.insert(dependency_package_name(key, value));
        } else if let Some(dependency) = &table_dependency
            && key == "package"
        {
            dependencies.remove(dependency);
            dependencies.insert(value.trim_matches(['"', '\'']).to_owned());
        }
    }

    ManifestInfo {
        name: name.expect("manifest has no [package] name"),
        dependencies,
    }
}

fn dependency_table_name(section: &str) -> Option<&str> {
    section
        .strip_prefix("dependencies.")
        .or_else(|| section.strip_prefix("build-dependencies."))
        .or_else(|| {
            section
                .starts_with("target.")
                .then(|| {
                    section
                        .rsplit_once(".dependencies.")
                        .or_else(|| section.rsplit_once(".build-dependencies."))
                        .map(|(_, name)| name)
                })
                .flatten()
        })
        .map(|name| name.trim_matches(['"', '\'']))
}

fn is_dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "build-dependencies"
        || (section.starts_with("target.")
            && (section.ends_with(".dependencies") || section.ends_with(".build-dependencies")))
}

fn dependency_package_name(key: &str, value: &str) -> String {
    value
        .trim_matches(['{', '}'])
        .split(',')
        .filter_map(|field| field.split_once('='))
        .find_map(|(field, value)| {
            (field.trim() == "package").then(|| value.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| key.split_once('.').map_or(key, |(name, _)| name).to_owned())
}

fn dependency_cycle(manifests: &BTreeMap<String, ManifestInfo>) -> Option<Vec<String>> {
    fn visit(
        name: &str,
        manifests: &BTreeMap<String, ManifestInfo>,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        path: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if let Some(start) = path.iter().position(|entry| entry == name) {
            let mut cycle = path[start..].to_vec();
            cycle.push(name.to_owned());
            return Some(cycle);
        }
        if visited.contains(name) {
            return None;
        }

        visiting.insert(name.to_owned());
        path.push(name.to_owned());
        if let Some(manifest) = manifests.get(name) {
            for dependency in &manifest.dependencies {
                if manifests.contains_key(dependency)
                    && let Some(cycle) = visit(dependency, manifests, visiting, visited, path)
                {
                    return Some(cycle);
                }
            }
        }
        path.pop();
        visiting.remove(name);
        visited.insert(name.to_owned());
        None
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for name in manifests.keys() {
        if let Some(cycle) = visit(
            name,
            manifests,
            &mut visiting,
            &mut visited,
            &mut Vec::new(),
        ) {
            return Some(cycle);
        }
    }
    None
}

fn workspace_manifests(root: &Path) -> BTreeMap<String, ManifestInfo> {
    crate_directories(root)
        .into_iter()
        .map(|directory| {
            let manifest = fs::read_to_string(directory.join("Cargo.toml"))
                .expect("read workspace crate manifest");
            let parsed = parse_manifest(&manifest);
            (parsed.name.clone(), parsed)
        })
        .collect()
}

fn allowed_workspace_dependencies(crate_name: &str) -> &'static [&'static str] {
    match crate_name {
        "studiolo-core" => &[],
        "studiolo-vault-broker"
        | "studiolo-vault-index"
        | "studiolo-control-db"
        | "studiolo-audit-ledger"
        | "studiolo-runner-adapter" => &["studiolo-core"],
        "studiolo-gateway" => &["studiolo-core", "studiolo-audit-ledger"],
        "studiolo-task-engine" => &[
            "studiolo-core",
            "studiolo-control-db",
            "studiolo-vault-broker",
            "studiolo-audit-ledger",
            "studiolo-runner-adapter",
        ],
        "studiolo" => &[
            "studiolo-core",
            "studiolo-vault-broker",
            "studiolo-vault-index",
            "studiolo-control-db",
            "studiolo-audit-ledger",
            "studiolo-runner-adapter",
            "studiolo-gateway",
            "studiolo-task-engine",
        ],
        other => panic!("new workspace crate {other} needs an explicit dependency policy"),
    }
}

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
    assert!(broker_source.contains("open_ambient_dir"));
    assert!(broker_source.contains(".workbench/vault.json"));
    assert!(broker_source.contains("sources/attempts/"));
}

#[test]
fn dependency_scanner_detects_a_sneaky_cycle_fixture() {
    let manifests = [
        r#"
            [package]
            name = "crate-a"
            [dependencies]
            crate-b = { path = "../b" }
        "#,
        r#"
            [package]
            name = "crate-b"
            [dependencies]
            renamed-a = { package = "crate-a", path = "../a" }
        "#,
    ];
    let parsed = manifests
        .into_iter()
        .map(parse_manifest)
        .map(|manifest| (manifest.name.clone(), manifest))
        .collect::<BTreeMap<_, _>>();

    assert!(dependency_cycle(&parsed).is_some());
}

#[test]
fn dependency_scanner_includes_renamed_target_and_table_dependencies() {
    let manifest = parse_manifest(
        r#"
        [package]
        name = "fixture"
        [dependencies.renamed-broker]
        package = "studiolo-vault-broker"
        path = "../vault-broker"
        [target.'cfg(unix)'.dependencies.renamed-core]
        package = "studiolo-core"
        path = "../core"
        [target.'cfg(unix)'.build-dependencies]
        native = { package = "rustix", version = "1" }
        [dev-dependencies]
        test-only = "1"
    "#,
    );
    assert_eq!(
        manifest.dependencies,
        ["studiolo-vault-broker", "studiolo-core", "rustix"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
}

#[test]
fn workspace_dependencies_follow_the_acyclic_module_direction() {
    let manifests = workspace_manifests(&workspace_root());
    assert_eq!(manifests.len(), 9, "workspace crate inventory changed");
    assert!(
        dependency_cycle(&manifests).is_none(),
        "workspace dependency cycle: {:?}",
        dependency_cycle(&manifests)
    );

    let workspace_names = manifests.keys().cloned().collect::<BTreeSet<_>>();
    for manifest in manifests.values() {
        let allowed = allowed_workspace_dependencies(&manifest.name)
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let unexpected = manifest
            .dependencies
            .iter()
            .filter(|dependency| workspace_names.contains(*dependency))
            .filter(|dependency| !allowed.contains(dependency.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            unexpected.is_empty(),
            "{} has forbidden workspace dependencies: {unexpected:?}",
            manifest.name
        );
    }

    for read_side in [
        "studiolo-vault-index",
        "studiolo-gateway",
        "studiolo-audit-ledger",
        "studiolo-runner-adapter",
    ] {
        assert!(
            !manifests[read_side]
                .dependencies
                .contains("studiolo-vault-broker"),
            "{read_side} must not acquire canonical Vault authority"
        );
    }
}

#[test]
fn filesystem_authority_dependencies_have_explicit_owners() {
    let manifests = workspace_manifests(&workspace_root());
    let canonical_filesystem_dependencies = ["cap-std", "cap-tempfile", "rustix"];

    for manifest in manifests.values() {
        let owned = canonical_filesystem_dependencies
            .iter()
            .filter(|dependency| manifest.dependencies.contains(**dependency))
            .copied()
            .collect::<Vec<_>>();
        if manifest.name == "studiolo-vault-broker" {
            assert_eq!(owned, canonical_filesystem_dependencies);
        } else {
            assert!(
                owned.is_empty(),
                "{} directly acquired canonical filesystem dependencies: {owned:?}",
                manifest.name
            );
        }
    }

    assert!(
        manifests["studiolo-control-db"]
            .dependencies
            .contains("rusqlite")
    );
    assert!(
        manifests["studiolo-runner-adapter"]
            .dependencies
            .contains("tempfile")
    );
    for forbidden in ["rusqlite", "tempfile", "cap-std", "cap-tempfile", "rustix"] {
        assert!(
            !manifests["studiolo-audit-ledger"]
                .dependencies
                .contains(forbidden),
            "M0 AuditLedger must remain in memory, found {forbidden}"
        );
    }
}

#[test]
fn ci_commands_lock_dependencies_and_run_linux_path_security_explicitly() {
    let root = workspace_root();
    for relative in [".github/workflows/ci.yml", "scripts/test-linux.sh"] {
        let content = fs::read_to_string(root.join(relative)).expect("read CI entrypoint");
        for command in [
            "cargo --locked fmt",
            "cargo --locked clippy",
            "cargo --locked test",
            "cargo --locked test -p studiolo-vault-broker --test linux_path_security",
        ] {
            assert!(
                content.contains(command),
                "{relative} is missing `{command}`"
            );
        }
    }
}
