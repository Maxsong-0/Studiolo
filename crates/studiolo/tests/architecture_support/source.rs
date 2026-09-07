use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthorityViolation {
    CanonicalPath,
    AmbientRoot,
    GenericWrite,
    DatabaseOpen,
}

pub(crate) fn source_violations(
    crate_name: &str,
    source_path: &Path,
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
    // 每个私有 Adapter 只创建自己登记的固定文件；运行时还须检查路径与身份。
    let checked_source = private_storage_remainder(crate_name, source_path, source);
    let checked_source = super::admin::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::unix_ingress::remainder(crate_name, source_path, &checked_source);
    let has_generic_write = generic_write_patterns
        .iter()
        .any(|pattern| checked_source.contains(pattern))
        || (matches!(crate_name, "studiolo-auth-admin" | "studiolo-host-config")
            && checked_source.contains("File::open("));
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
        checked_source.contains("fs::")
    };
    if (has_generic_write && !runner_writes_only_its_temp_result) || imports_filesystem_authority {
        violations.push(AuthorityViolation::GenericWrite);
    }
    if let Some((_, connection_path, _)) = private_storage_policy(crate_name)
        && source.contains("Connection::open")
    {
        let compact = source.split_whitespace().collect::<String>();
        if !source_path.ends_with(connection_path)
            || compact.matches("Connection::open").count() != 1
            || !compact.contains("Connection::open_with_flags(&storage.database,flags)")
        {
            violations.push(AuthorityViolation::DatabaseOpen);
        }
    }
    violations
}

fn private_storage_policy(crate_name: &str) -> Option<(&str, &str, &str)> {
    match crate_name {
        "studiolo-audit-ledger" => Some((
            "crates/audit-ledger/src/sqlite/storage.rs",
            "crates/audit-ledger/src/sqlite.rs",
            "audit.sqlite3",
        )),
        "studiolo-auth" => Some((
            "crates/auth/src/session/storage.rs",
            "crates/auth/src/session/database.rs",
            "auth.sqlite3",
        )),
        _ => None,
    }
}

fn private_storage_remainder(crate_name: &str, path: &Path, source: &str) -> String {
    let Some((storage_path, _, database_name)) = private_storage_policy(crate_name) else {
        return source.to_owned();
    };
    if !path.ends_with(storage_path) {
        return source.to_owned();
    }
    let mut remainder = source.split_whitespace().collect::<String>();
    if remainder.matches("database=").count() != 1
        || !remainder.contains(&format!("letdatabase=root.join(\"{database_name}\");"))
        || remainder.matches("OpenOptions::new()").count() != 1
    {
        return remainder;
    }
    for allowed in [
        "std::fs::Metadata",
        "os::unix::fs::",
        "fs::{self,OpenOptions}",
        "fs::symlink_metadata(",
        "fs::File::open(root)",
        "OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(&database)",
        "file.sync_all()",
    ] {
        remainder = remainder.replace(allowed, "");
    }
    remainder
}
