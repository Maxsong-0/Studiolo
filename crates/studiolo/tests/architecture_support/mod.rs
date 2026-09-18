mod admin;
mod backup;
mod maintenance;
mod manifest;
mod observability;
mod policy;
mod source;
mod sync_bridge;
mod sync_transport;
mod unix_ingress;

pub(crate) use manifest::{dependency_cycle, parse_manifest, workspace_manifests};
pub(crate) use policy::allowed_workspace_dependencies;
pub(crate) use source::{AuthorityViolation, source_violations};

use std::{
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
