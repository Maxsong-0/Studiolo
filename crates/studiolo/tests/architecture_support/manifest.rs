use super::crate_directories;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManifestInfo {
    pub(crate) name: String,
    pub(crate) dependencies: BTreeSet<String>,
}

pub(crate) fn parse_manifest(manifest: &str) -> ManifestInfo {
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

pub(crate) fn dependency_table_name(section: &str) -> Option<&str> {
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

pub(crate) fn is_dependency_section(section: &str) -> bool {
    section == "dependencies"
        || section == "build-dependencies"
        || (section.starts_with("target.")
            && (section.ends_with(".dependencies") || section.ends_with(".build-dependencies")))
}

pub(crate) fn dependency_package_name(key: &str, value: &str) -> String {
    value
        .trim_matches(['{', '}'])
        .split(',')
        .filter_map(|field| field.split_once('='))
        .find_map(|(field, value)| {
            (field.trim() == "package").then(|| value.trim().trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| key.split_once('.').map_or(key, |(name, _)| name).to_owned())
}

pub(crate) fn dependency_cycle(manifests: &BTreeMap<String, ManifestInfo>) -> Option<Vec<String>> {
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

pub(crate) fn workspace_manifests(root: &Path) -> BTreeMap<String, ManifestInfo> {
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
