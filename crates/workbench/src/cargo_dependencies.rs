//! 从精确、完整的清单集合派生内部边；任何错误都丢弃整图。

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::TableLike;

use crate::{
    CargoDependency, CargoDependencyError as Error, CargoDependencyGraph, CargoManifest,
    CargoPackage,
    cargo_manifest::{Parsed, identifier, normalize, optional_table},
    cargo_plan,
    cargo_spec::{self, Definition, Spec},
};

/// 输入必须与根清单的显式读取计划完全相同；本函数不做任何外部读取。
pub fn derive_cargo_dependencies(
    manifests: &[CargoManifest],
) -> Result<CargoDependencyGraph, Error> {
    if manifests.len() > 65 {
        return Err(Error::unsupported("Cargo 清单数量超过预算"));
    }
    let mut inputs = BTreeMap::new();
    let mut bytes = 0_usize;
    for manifest in manifests {
        bytes = bytes.saturating_add(manifest.content.len());
        if bytes > 2 * 1024 * 1024 || manifest.content.len() > 64 * 1024 {
            return Err(Error::unsupported("Cargo 清单正文超过字节预算"));
        }
        if inputs.insert(manifest.path.as_str(), manifest).is_some() {
            return Err(Error::invalid("Cargo 清单输入包含重复路径"));
        }
    }
    let root = inputs
        .get("Cargo.toml")
        .ok_or_else(|| Error::invalid("Cargo 清单输入缺少根清单"))?;
    let root = Parsed::new("Cargo.toml", &root.content)?;
    let paths = cargo_plan::plan(&root)?;
    if !paths.iter().map(String::as_str).eq(inputs.keys().copied()) {
        return Err(Error::invalid("Cargo 清单输入与显式成员计划不完全一致"));
    }
    let parsed = inputs
        .values()
        .map(|manifest| Parsed::new(&manifest.path, &manifest.content))
        .collect::<Result<Vec<_>, _>>()?;
    let mut packages = BTreeMap::new();
    let mut names = BTreeSet::new();
    for manifest in &parsed {
        manifest.reject_overrides()?;
        match manifest.package()? {
            Some(package) => {
                if !names.insert(package.name.clone()) {
                    return Err(Error::invalid("Cargo 包名重复"));
                }
                packages.insert(package.id.clone(), package);
            }
            None if manifest.path == "Cargo.toml" => {}
            None => return Err(Error::invalid("Cargo 显式成员缺少 package")),
        }
    }
    let definitions = cargo_spec::definitions(&root)?;
    let mut graph = Builder {
        packages: &packages,
        definitions,
        dependencies: Vec::new(),
        external: 0,
        entries: 0,
        identities: BTreeSet::new(),
    };
    for manifest in &parsed {
        // 虚拟根不能隐含一个不存在的依赖方。
        let has_package = packages.contains_key(&manifest.directory);
        graph.sections(manifest, manifest.document.as_table(), None, has_package)?;
        if let Some(targets) = manifest.table("target")? {
            for (target, item) in targets.iter() {
                if target.len() > 256 {
                    return Err(Error::unsupported("Cargo target 条件超过字节预算"));
                }
                if target.trim().is_empty()
                    || target
                        .chars()
                        .any(|value| value.is_control() || matches!(value, '\u{2028}' | '\u{2029}'))
                {
                    return Err(Error::invalid("Cargo target 条件必须是有界非空单行文本"));
                }
                let target_table = optional_table(Some(item))?
                    .ok_or_else(|| Error::invalid("Cargo target 字段类型错误"))?;
                graph.sections(manifest, target_table, Some(target), has_package)?;
            }
        }
    }
    graph.dependencies.sort_by(|left, right| {
        (&left.from, &left.to, &left.kind, &left.target, &left.alias).cmp(&(
            &right.from,
            &right.to,
            &right.kind,
            &right.target,
            &right.alias,
        ))
    });
    Ok(CargoDependencyGraph {
        packages: packages.values().cloned().collect(),
        dependencies: graph.dependencies,
        external_dependencies: graph.external,
    })
}

type Identity = (String, String, String, String, Option<String>);

struct Builder<'a> {
    packages: &'a BTreeMap<String, CargoPackage>,
    definitions: BTreeMap<&'a str, Definition<'a>>,
    dependencies: Vec<CargoDependency>,
    external: u32,
    entries: usize,
    identities: BTreeSet<Identity>,
}

impl Builder<'_> {
    fn sections(
        &mut self,
        manifest: &Parsed<'_>,
        table: &dyn TableLike,
        target: Option<&str>,
        has_package: bool,
    ) -> Result<(), Error> {
        if ["dev_dependencies", "build_dependencies"]
            .iter()
            .any(|key| table.contains_key(key))
        {
            return Err(Error::unsupported("Cargo 依赖章节不支持旧式下划线拼写"));
        }
        for (section, kind) in [
            ("dependencies", "normal"),
            ("dev-dependencies", "dev"),
            ("build-dependencies", "build"),
        ] {
            if let Some(dependencies) = optional_table(table.get(section))? {
                if !has_package && !dependencies.is_empty() {
                    return Err(Error::invalid("虚拟根不能声明包依赖"));
                }
                for (alias, item) in dependencies.iter() {
                    if self.entries >= 2048 {
                        return Err(Error::unsupported("Cargo 依赖条目超过预算"));
                    }
                    self.entries += 1;
                    identifier(alias)?;
                    let (spec, base, inherited_from) =
                        if let Some(optional) = cargo_spec::inherited(item)? {
                            let definition = self
                                .definitions
                                .get(alias)
                                .ok_or_else(|| Error::invalid("Cargo 继承依赖没有根定义"))?;
                            (
                                Spec {
                                    optional,
                                    ..definition.spec
                                },
                                ".",
                                Some(definition.source.clone()),
                            )
                        } else {
                            (
                                cargo_spec::direct(item, false)?,
                                manifest.directory.as_str(),
                                None,
                            )
                        };
                    let Some(path) = spec.path else {
                        self.external += 1;
                        continue;
                    };
                    let to = normalize(base, path)?;
                    let package = self
                        .packages
                        .get(&to)
                        .ok_or_else(|| Error::unsupported("Cargo path 依赖超出显式成员集合"))?;
                    if package.name != spec.package.unwrap_or(alias) {
                        return Err(Error::invalid("Cargo path 目标包名与依赖声明不一致"));
                    }
                    if self.dependencies.len() >= 256 {
                        return Err(Error::unsupported("Cargo 内部依赖边超过预算"));
                    }
                    let identity = (
                        manifest.directory.clone(),
                        to.clone(),
                        alias.to_owned(),
                        kind.to_owned(),
                        target.map(str::to_owned),
                    );
                    if !self.identities.insert(identity) {
                        return Err(Error::invalid("Cargo 内部依赖身份重复"));
                    }
                    self.dependencies.push(CargoDependency {
                        from: manifest.directory.clone(),
                        to,
                        alias: alias.to_owned(),
                        kind: kind.to_owned(),
                        target: target.map(str::to_owned),
                        optional: spec.optional,
                        source: manifest.source(dependencies, alias)?,
                        inherited_from,
                    });
                }
            }
        }
        Ok(())
    }
}
