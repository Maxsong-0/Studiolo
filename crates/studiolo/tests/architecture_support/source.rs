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
        "File::options(",
        "OpenOptions::new(",
        ".write_all(",
        ".sync_all(",
        ".create_dir(",
        ".create_dir_all(",
        ".rename(",
        ".remove_file(",
        ".remove_dir(",
        "Command::new(",
        "TcpStream::connect",
    ];
    // 每个私有 Adapter 只创建自己登记的固定文件；运行时还须检查路径与身份。
    let checked_source = private_storage_remainder(crate_name, source_path, source);
    let checked_source = super::admin::remainder(crate_name, source_path, &checked_source);
    let checked_source = vault_admin_remainder(crate_name, source_path, &checked_source);
    let checked_source = super::unix_ingress::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::maintenance::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::sync_bridge::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::backup::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::observability::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::sync_transport::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::board_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::gateway_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::runner_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::executor_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::cleanup_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::real_fixture_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::snapshot_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source = super::demo_authority::remainder(crate_name, source_path, &checked_source);
    let checked_source =
        super::runner_fixture_authority::remainder(crate_name, source_path, &checked_source);
    if [".workbench/vault.json", "sources/attempts/"]
        .iter()
        .any(|pattern| checked_source.contains(pattern))
    {
        violations.push(AuthorityViolation::CanonicalPath);
    }
    if matches!(crate_name, "studiolo-gateway" | "studiolo")
        && checked_source.contains("Connection::open")
    {
        violations.push(AuthorityViolation::DatabaseOpen);
    }
    let has_generic_write = generic_write_patterns
        .iter()
        .any(|pattern| checked_source.contains(pattern))
        || (matches!(crate_name, "studiolo-auth-admin" | "studiolo-host-config")
            && checked_source.contains("File::open("));
    let runner_writes_only_its_temp_result = if crate_name == "studiolo-runner-adapter"
        && source_path.ends_with("crates/runner-adapter/src/lib.rs")
    {
        let mut remainder = checked_source.to_owned();
        for allowed in [
            "File::create(&result_path)",
            "file.write_all(",
            "file.sync_all(",
        ] {
            remainder = remainder.replace(allowed, "");
        }
        checked_source.contains("temp_dir.path().join(\"result.md\")")
            && [
                "File::create(&result_path)",
                "file.write_all(",
                "file.sync_all(",
            ]
            .iter()
            .all(|call| checked_source.matches(call).count() == 1)
            && !generic_write_patterns
                .iter()
                .any(|pattern| remainder.contains(pattern))
    } else {
        false
    };
    let imports_filesystem_authority = if crate_name == "studiolo-runner-adapter" {
        if source_path.ends_with("crates/runner-adapter/src/lib.rs") {
            checked_source.replacen("fs::File,", "", 1).contains("fs::")
        } else {
            checked_source.contains("fs::")
        }
    } else {
        checked_source.contains("fs::")
    };
    if (has_generic_write && !runner_writes_only_its_temp_result) || imports_filesystem_authority {
        violations.push(AuthorityViolation::GenericWrite);
    }
    if let Some((_, connection_path, _)) = private_storage_policy(crate_name, source_path)
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

fn vault_admin_remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name == "studiolo-vault-admin" && path.ends_with("crates/vault-admin/src/main.rs") {
        return source.replace(
            "rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o077));",
            "",
        );
    }
    if crate_name != "studiolo-vault-admin" || !path.ends_with("crates/vault-admin/src/source.rs") {
        return source.to_owned();
    }
    let mut remainder = source.split_whitespace().collect::<String>();
    let descriptor = "open(path,OFlags::RDONLY|OFlags::NOFOLLOW|OFlags::NONBLOCK|OFlags::CLOEXEC,Mode::empty(),)";
    if remainder.matches(descriptor).count() != 1
        || !remainder.contains(
            ".canonical_serving_roots(&options.vault,&options.assets,&options.writer_data)",
        )
        || !remainder.contains("ReadOnlyVault::open_private(path)")
        || !remainder.contains("prepared.empty_targets()?;")
        || !remainder.contains("ConfigRevision::read(&self.options.config)?!=self.revision")
    {
        return remainder;
    }
    // 只允许路径/权限复核与配置的只读 nofollow FD；正文始终来自 ReadOnlyVault。
    for pattern in [
        "rustix::fs::{Mode,OFlags,open}",
        "fs::{self,File}",
        "os::unix::fs::MetadataExt",
        "fs::Metadata",
        "fs::symlink_metadata(",
        "fs::read_dir(",
        descriptor,
    ] {
        remainder = remainder.replace(pattern, "");
    }
    if remainder.contains("open(") || remainder.contains("OFlags::") {
        // Pin::open 是固定只读适配器，不允许添第二个环境 FD 打开点。
        remainder = remainder
            .replace("Self::open(", "")
            .replace("fnopen(", "")
            .replace("ReadOnlyVault::open(", "");
        if remainder.contains("open(") || remainder.contains("OFlags::") {
            return format!("fs::unexpected {remainder}");
        }
    }
    remainder
}

fn private_storage_policy(
    crate_name: &str,
    path: &Path,
) -> Option<(&'static str, &'static str, &'static str)> {
    match crate_name {
        "studiolo-sync-bridge" => Some((
            "crates/sync-bridge/src/sqlite/storage.rs",
            "crates/sync-bridge/src/sqlite/opening.rs",
            "bridge.sqlite3",
        )),
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
        "studiolo-supply"
            if path.ends_with("crates/supply/src/index_cache/paths.rs")
                || path.ends_with("crates/supply/src/index_cache/mod.rs") =>
        {
            Some((
                "crates/supply/src/index_cache/paths.rs",
                "crates/supply/src/index_cache/mod.rs",
                "index.sqlite3",
            ))
        }
        "studiolo-supply" => Some((
            "crates/supply/src/storage.rs",
            "crates/supply/src/database.rs",
            "agent.sqlite3",
        )),
        // M4 策略平面：第六个固定私有库文件名；主人与接入方句柄共用同一个打开点。
        "studiolo-policy" => Some((
            "crates/policy/src/storage.rs",
            "crates/policy/src/database.rs",
            "policy.sqlite3",
        )),
        _ => None,
    }
}

fn private_storage_remainder(crate_name: &str, path: &Path, source: &str) -> String {
    let Some((storage_path, _, database_name)) = private_storage_policy(crate_name, path) else {
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
    if path.ends_with("crates/supply/src/index_cache/paths.rs") {
        remainder = remainder.replace("fs::canonicalize(", "");
    }
    if path.ends_with("crates/supply/src/storage.rs")
        || path.ends_with("crates/policy/src/storage.rs")
    {
        // 只保留固定私有库新建句柄，不能借此新增打开或通用写入。
        remainder = remainder.replace("created_pin:Option<std::fs::File>,", "");
    }
    remainder
}
