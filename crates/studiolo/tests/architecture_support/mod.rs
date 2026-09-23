mod admin;
mod authority_tokens;
mod backup;
mod board_authority;
mod cleanup_authority;
mod demo_authority;
mod executor_authority;
mod gateway_authority;
mod maintenance;
mod manifest;
mod observability;
mod policy;
mod real_fixture_authority;
mod restore_authority;
mod runner_authority;
mod runner_fixture_authority;
pub(crate) mod rust_lexer;
mod snapshot_authority;
mod source;
mod sync_bridge;
mod sync_transport;
mod unix_ingress;

pub(crate) use manifest::{dependency_cycle, parse_manifest, workspace_manifests};
pub(crate) use policy::allowed_workspace_dependencies;
pub(crate) use source::{AuthorityViolation, source_violations};

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("studiolo crate lives at <workspace>/crates/studiolo")
        .to_path_buf()
}

pub(crate) fn crate_directories(root: &Path) -> Vec<PathBuf> {
    let mut directories = fs::read_dir(root.join("crates"))
        .expect("read workspace crates")
        .map(|entry| entry.expect("read crate directory entry").path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .collect::<Vec<_>>();
    directories.sort();
    directories
}

/// `package` 连同它经 `[dependencies]`／`[build-dependencies]`（含目标平台限定表与改名依赖）
/// 传递可达的全部工作区 crate 目录，按路径排序；`[dev-dependencies]` 不进产物，不算。
/// 依赖表沿用依赖方向守卫的同一份清单解析（`parse_manifest`），不另起一套。
pub(crate) fn workspace_dependency_closure(root: &Path, package: &str) -> Vec<PathBuf> {
    let manifests = workspace_manifests(root);
    assert!(manifests.contains_key(package), "{package} 不是工作区成员");
    let directories = crate_directories(root)
        .into_iter()
        .map(|directory| {
            let manifest = fs::read_to_string(directory.join("Cargo.toml"))
                .expect("read workspace crate manifest");
            (package_name(&manifest), directory)
        })
        .collect::<BTreeMap<_, _>>();
    let (mut pending, mut reached) = (vec![package], BTreeSet::new());
    while let Some(name) = pending.pop() {
        if reached.insert(name) {
            pending.extend(
                manifests[name]
                    .dependencies
                    .iter()
                    .map(String::as_str)
                    .filter(|dependency| manifests.contains_key(*dependency)),
            );
        }
    }
    let mut closure = reached
        .into_iter()
        .map(|name| directories[name].clone())
        .collect::<Vec<_>>();
    closure.sort();
    closure
}

pub(crate) fn rust_sources(root: &Path) -> Vec<PathBuf> {
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

pub(crate) fn package_name(manifest: &str) -> String {
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
