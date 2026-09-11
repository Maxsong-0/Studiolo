//! 校验依赖声明类型和来源语义，不解析版本、feature 或条件是否启用。

use std::collections::BTreeMap;

use toml_edit::{Item, TableLike};

use crate::{
    CargoDependencyError as Error, ManifestSource,
    cargo_manifest::{Parsed, identifier, optional_table, string},
};

#[derive(Clone, Copy)]
pub(crate) struct Spec<'a> {
    pub(crate) package: Option<&'a str>,
    pub(crate) path: Option<&'a str>,
    pub(crate) optional: bool,
}

pub(crate) struct Definition<'a> {
    pub(crate) spec: Spec<'a>,
    pub(crate) source: ManifestSource,
}

/// 根模板只解析一次；未使用模板不计入包依赖条目，但仍拒绝未知语义。
pub(crate) fn definitions<'a>(
    root: &'a Parsed<'_>,
) -> Result<BTreeMap<&'a str, Definition<'a>>, Error> {
    let mut result = BTreeMap::new();
    if let Some(workspace) = root.table("workspace")?
        && let Some(definitions) = optional_table(workspace.get("dependencies"))?
    {
        for (alias, item) in definitions.iter() {
            if result.len() >= 2048 {
                return Err(Error::unsupported("Cargo workspace 依赖定义超过预算"));
            }
            identifier(alias)?;
            let spec = direct(item, true)?;
            result.insert(
                alias,
                Definition {
                    spec,
                    source: root.source(definitions, alias)?,
                },
            );
        }
    }
    Ok(result)
}

pub(crate) fn direct(item: &Item, workspace_definition: bool) -> Result<Spec<'_>, Error> {
    if item.is_str() {
        string(Some(item))?;
        return Ok(Spec {
            package: None,
            path: None,
            optional: false,
        });
    }
    let table = item
        .as_table_like()
        .ok_or_else(|| Error::invalid("Cargo 依赖必须是版本字符串或表"))?;
    let allowed = [
        "package",
        "path",
        "version",
        "git",
        "registry",
        "branch",
        "tag",
        "rev",
        "optional",
        "features",
        "default-features",
    ];
    if table.iter().any(|(key, _)| !allowed.contains(&key)) {
        return Err(Error::unsupported("Cargo 依赖含不支持的语义字段"));
    }
    if workspace_definition && table.contains_key("optional") {
        return Err(Error::invalid("workspace.dependencies 不允许 optional"));
    }
    let optional = boolean(table, "optional")?.unwrap_or(false);
    boolean(table, "default-features")?;
    features(table)?;
    let package = optional_string(table, "package")?;
    if let Some(package) = package {
        identifier(package)?;
    }
    let path = optional_string(table, "path")?;
    let git = optional_string(table, "git")?;
    let version = optional_string(table, "version")?;
    let registry = optional_string(table, "registry")?;
    let mut selectors = 0;
    for selector in ["branch", "tag", "rev"] {
        selectors += usize::from(optional_string(table, selector)?.is_some());
    }
    if selectors > 1
        || (selectors > 0 && git.is_none())
        || (git.is_some() && (path.is_some() || registry.is_some()))
        || (path.is_none() && git.is_none() && version.is_none())
    {
        return Err(Error::invalid("Cargo 依赖来源缺失或互相矛盾"));
    }
    Ok(Spec {
        package,
        path,
        optional,
    })
}

/// Some 表示 workspace 继承及成员自己的 optional；None 表示直接定义。
pub(crate) fn inherited(item: &Item) -> Result<Option<bool>, Error> {
    let Some(table) = item.as_table_like() else {
        return Ok(None);
    };
    let Some(workspace) = table.get("workspace") else {
        return Ok(None);
    };
    if workspace.as_bool() != Some(true) {
        return Err(Error::invalid("Cargo workspace 继承必须明确为 true"));
    }
    if table
        .iter()
        .any(|(key, _)| !["workspace", "optional", "features"].contains(&key))
    {
        return Err(Error::unsupported(
            "Cargo 继承依赖只能追加 optional 和 features",
        ));
    }
    features(table)?;
    Ok(Some(boolean(table, "optional")?.unwrap_or(false)))
}

fn optional_string<'a>(table: &'a dyn TableLike, key: &str) -> Result<Option<&'a str>, Error> {
    table.get(key).map(|item| string(Some(item))).transpose()
}

fn boolean(table: &dyn TableLike, key: &str) -> Result<Option<bool>, Error> {
    table
        .get(key)
        .map(|item| {
            item.as_bool()
                .ok_or_else(|| Error::invalid("Cargo 依赖布尔字段类型错误"))
        })
        .transpose()
}

fn features(table: &dyn TableLike) -> Result<(), Error> {
    if let Some(features) = table.get("features") {
        let features = features
            .as_array()
            .ok_or_else(|| Error::invalid("Cargo features 必须是字符串数组"))?;
        if features.iter().any(|value| {
            value
                .as_str()
                .is_none_or(|value| value.is_empty() || value.chars().any(char::is_control))
        }) {
            return Err(Error::invalid("Cargo features 必须逐项提供单行字符串"));
        }
    }
    Ok(())
}
