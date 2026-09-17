//! 不可变 TOML、规范路径与原文位置；所有错误均在这里改为受控原因。

use std::collections::BTreeMap;

use studiolo_core::VaultPath;
use toml_edit::{Document, Item, TableLike};

use crate::{CargoDependencyError as Error, CargoPackage, ManifestSource, operations_parse};

pub(crate) struct Parsed<'a> {
    pub(crate) path: &'a str,
    pub(crate) directory: String,
    pub(crate) document: Document<&'a str>,
    line_starts: Vec<usize>,
}

impl<'a> Parsed<'a> {
    pub(crate) fn new(path: &'a str, content: &'a str) -> Result<Self, Error> {
        if content.len() > 64 * 1024 {
            return Err(Error::unsupported("Cargo 清单超过单文件字节预算"));
        }
        let document = Document::parse(content)
            .map_err(|_| Error::invalid("Cargo 清单不是有效的受支持 TOML"))?;
        let mut line_starts = vec![0];
        line_starts.extend(
            content
                .bytes()
                .enumerate()
                .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
        );
        Ok(Self {
            path,
            directory: path
                .rsplit_once('/')
                .map_or(".", |(directory, _)| directory)
                .to_owned(),
            document,
            line_starts,
        })
    }

    pub(crate) fn table(&self, key: &str) -> Result<Option<&dyn TableLike>, Error> {
        optional_table(self.document.get(key))
    }

    pub(crate) fn source(&self, table: &dyn TableLike, key: &str) -> Result<ManifestSource, Error> {
        let span = table
            .key(key)
            .and_then(|key| key.span())
            .or_else(|| table.get(key).and_then(Item::span))
            .ok_or_else(|| Error::invalid("Cargo 清单缺少可验证的原文位置"))?;
        if span.start > span.end || span.end > self.document.raw().len() {
            return Err(Error::invalid("Cargo 清单原文位置越界"));
        }
        Ok(ManifestSource {
            path: self.path.to_owned(),
            line: self
                .line_starts
                .partition_point(|offset| *offset <= span.start),
        })
    }

    pub(crate) fn package(&self) -> Result<Option<CargoPackage>, Error> {
        let Some(package) = self.table("package")? else {
            return Ok(None);
        };
        let name = string(package.get("name"))?;
        identifier(name)?;
        if let Some(workspace) = package.get("workspace")
            && normalize(&self.directory, string(Some(workspace))?)? != "."
        {
            return Err(Error::unsupported("package.workspace 只能指向当前绑定根"));
        }
        Ok(Some(CargoPackage {
            id: self.directory.clone(),
            name: name.to_owned(),
            source: self.source(package, "name")?,
        }))
    }

    pub(crate) fn reject_overrides(&self) -> Result<(), Error> {
        for key in ["patch", "replace"] {
            if self.table(key)?.is_some_and(|table| !table.is_empty()) {
                return Err(Error::unsupported("不支持非空 patch 或 replace 依赖覆盖"));
            }
        }
        if self.path != "Cargo.toml" && self.document.contains_key("workspace") {
            return Err(Error::unsupported("不支持成员中的嵌套 workspace"));
        }
        Ok(())
    }
}

pub(crate) fn optional_table(item: Option<&Item>) -> Result<Option<&dyn TableLike>, Error> {
    item.map(|item| {
        item.as_table_like()
            .ok_or_else(|| Error::invalid("Cargo 清单表字段类型错误"))
    })
    .transpose()
}

pub(crate) fn string(item: Option<&Item>) -> Result<&str, Error> {
    item.and_then(Item::as_str)
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .ok_or_else(|| Error::invalid("Cargo 清单字符串字段缺失或类型错误"))
}

pub(crate) fn identifier(value: &str) -> Result<(), Error> {
    if value.len() > 64 {
        return Err(Error::unsupported("Cargo 包名或依赖别名超过字节预算"));
    }
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(Error::invalid("Cargo 包名或依赖别名必须是受限 ASCII 标识"));
    }
    Ok(())
}

/// 仅作词法归一，不读取目录；最终给 Broker 的路径绝不含点段。
pub(crate) fn normalize(base: &str, raw: &str) -> Result<String, Error> {
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
        || (raw.as_bytes().get(1) == Some(&b':') && raw.as_bytes()[0].is_ascii_alphabetic())
    {
        return Err(Error::invalid("Cargo 路径必须是根内可见的相对路径"));
    }
    let mut components: Vec<&str> = if base == "." {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for component in raw.split('/') {
        match component {
            "." => {}
            ".." => {
                components
                    .pop()
                    .ok_or_else(|| Error::unsupported("Cargo 路径超出绑定根范围"))?;
            }
            "" => return Err(Error::invalid("Cargo 路径含空分量")),
            value if value.starts_with('.') => {
                return Err(Error::invalid("Cargo 路径不允许隐藏分量"));
            }
            value => components.push(value),
        }
    }
    if components.is_empty() {
        return Ok(".".to_owned());
    }
    if components.len() > 16 {
        return Err(Error::unsupported("Cargo 包目录超过分量预算"));
    }
    let path = components.join("/");
    if path.len() > 1024 {
        return Err(Error::unsupported("Cargo 包目录超过字节预算"));
    }
    if !operations_parse::repository_path(&path) {
        return Err(Error::invalid("Cargo 路径不是规范可见的 VaultPath"));
    }
    Ok(path)
}

pub(crate) fn directory_collision(
    directory: &str,
    prefixes: &mut BTreeMap<String, String>,
) -> Result<(), Error> {
    let mut prefix = String::new();
    for part in directory.split('/') {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        let key = if prefix == "." {
            prefix.clone()
        } else {
            VaultPath::parse(&prefix)
                .map_err(|_| Error::invalid("Cargo 目录路径无效"))?
                .collision_key()
        };
        if prefixes
            .get(&key)
            .is_some_and(|existing| existing != &prefix)
        {
            return Err(Error::invalid("Cargo 成员目录存在大小写名称碰撞"));
        }
        prefixes.insert(key, prefix.clone());
    }
    Ok(())
}
