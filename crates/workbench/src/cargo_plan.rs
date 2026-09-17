//! 先完整验证显式成员集合，再允许采集器读取任何成员清单。

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    CargoDependencyError as Error,
    cargo_manifest::{Parsed, directory_collision, normalize, optional_table},
};

/// 从根清单正文获得完整、排序、唯一的相对 Cargo.toml 读取计划。
/// 不展开 glob，不发现隐式成员，也不跟随集合外 path 依赖。
pub fn cargo_manifest_paths(root: &str) -> Result<Vec<String>, Error> {
    let root = Parsed::new("Cargo.toml", root)?;
    plan(&root)
}

pub(crate) fn plan(root: &Parsed<'_>) -> Result<Vec<String>, Error> {
    root.reject_overrides()?;
    let root_package = root.package()?.is_some();
    let mut packages = BTreeSet::new();
    let mut prefixes = BTreeMap::new();
    if root_package {
        packages.insert(".".to_owned());
    }
    if let Some(workspace) = root.table("workspace")? {
        optional_table(workspace.get("dependencies"))?;
        if let Some(exclude) = workspace.get("exclude") {
            let exclude = exclude
                .as_array()
                .ok_or_else(|| Error::invalid("workspace.exclude 必须是数组"))?;
            if !exclude.is_empty() {
                return Err(Error::unsupported("不支持非空 workspace.exclude"));
            }
        }
        if let Some(members) = workspace.get("members") {
            let members = members
                .as_array()
                .ok_or_else(|| Error::invalid("workspace.members 必须是数组"))?;
            let mut declared = BTreeSet::new();
            for member in members.iter() {
                let member = member
                    .as_str()
                    .ok_or_else(|| Error::invalid("workspace.members 必须逐项声明目录字符串"))?;
                if member.contains(['*', '?', '[', ']', '{', '}']) {
                    return Err(Error::unsupported("workspace.members 不支持 glob 成员"));
                }
                let directory = normalize(".", member)?;
                if !declared.insert(directory.clone()) {
                    return Err(Error::invalid("workspace.members 包含重复目录"));
                }
                directory_collision(&directory, &mut prefixes)?;
                if packages.len() >= 64 && !packages.contains(&directory) {
                    return Err(Error::unsupported("Cargo 包数量超过预算"));
                }
                packages.insert(directory);
            }
        }
    }
    if packages.is_empty() {
        return Err(Error::invalid("Cargo 清单没有根包或显式成员包"));
    }
    if packages.contains(".") && !root_package {
        return Err(Error::invalid("虚拟根不能作为没有 package 的成员"));
    }
    let mut paths = BTreeSet::from(["Cargo.toml".to_owned()]);
    paths.extend(
        packages
            .into_iter()
            .filter(|path| path != ".")
            .map(|path| format!("{path}/Cargo.toml")),
    );
    Ok(paths.into_iter().collect())
}
