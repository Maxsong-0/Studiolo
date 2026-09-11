//! 有界目录树声明解析；缩进、路径或覆盖范围含糊时整块拒绝。

use std::collections::{BTreeMap, BTreeSet};

use studiolo_core::VaultPath;

use crate::{DirectoryDeclaration, markdown, operations_block::DeclarationBlock, tree_validate};

type Parsed = (Vec<DirectoryDeclaration>, Vec<String>);

pub(crate) fn parse(source: &str, block: &DeclarationBlock<'_>) -> Result<Parsed, String> {
    let mut lines = block.lines.iter().filter(|(_, raw)| !raw.trim().is_empty());
    let Some((_, root)) = lines.next() else {
        return Err("目录声明块为空".to_owned());
    };
    let (root, _) = split_label(root.trim());
    let Some(root) = root.strip_suffix('/') else {
        return Err("目录树缺少明确仓库根".to_owned());
    };
    if !declaration_path(root) || root.contains('/') {
        return Err("仓库根必须是单个可见目录名".to_owned());
    }
    let mut state = Parser {
        source,
        nodes: BTreeMap::new(),
        collision_keys: BTreeSet::new(),
        parents: BTreeSet::new(),
    };
    let mut stack = vec![Some(String::new())];
    let mut entries = 0_usize;
    for (number, raw) in lines {
        let (depth, entry) =
            branch(raw).ok_or_else(|| format!("第 {number} 行目录树缩进或分支含糊"))?;
        if depth > stack.len() {
            return Err(format!("第 {number} 行目录树缩进跳层"));
        }
        stack.truncate(depth);
        let parent = stack
            .last()
            .and_then(Option::as_deref)
            .ok_or_else(|| format!("第 {number} 行没有明确的父目录"))?;
        let (name, label) = split_label(entry.trim());
        let directory = name.ends_with('/');
        let name = name.strip_suffix('/').unwrap_or(name);
        if !directory && hidden_file(name) {
            // 隐藏文件不比较；保留它之前已明确出现的可见父路径。
            let visible = name
                .split('/')
                .take_while(|part| !part.starts_with('.'))
                .collect::<Vec<_>>()
                .join("/");
            if !visible.is_empty() {
                let path = if parent.is_empty() {
                    visible
                } else {
                    format!("{parent}/{visible}")
                };
                if !declaration_path(&path) {
                    return Err(format!("第 {number} 行隐藏文件的可见父路径不可用"));
                }
                state.insert_path(&path, None, false)?;
            }
            if state.nodes.len() > 256 {
                return Err("目录声明超过 256 个目录，整块不可用".to_owned());
            }
            stack.push(None);
            continue;
        }
        if !declaration_path(name) || label.chars().any(char::is_control) {
            return Err(format!(
                "第 {number} 行目录路径或职责包含不安全、隐藏或模糊声明"
            ));
        }
        let path = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        };
        if !declaration_path(&path) {
            return Err(format!("第 {number} 行目录路径不可用"));
        }
        if directory {
            state.insert_path(&path, Some((*number, label)), true)?;
            stack.push(Some(path));
        } else {
            let parent_path = tree_validate::parent(&path);
            if !parent_path.is_empty() {
                state.insert_path(parent_path, None, false)?;
            }
            stack.push(None);
        }
        // 文件可证明所在树层展开，但不宣称多段文件路径内的各层完整。
        if depth == 1 {
            state.parents.insert(String::new());
        }
        entries += 1;
        if state.nodes.len() > 256 || state.parents.len() > 64 {
            return Err("目录声明超过 256 个目录或 64 个比较父层，整块不可用".to_owned());
        }
    }
    if entries == 0 {
        return Err("目录树没有可识别条目".to_owned());
    }
    Ok((
        state.nodes.into_values().collect(),
        state.parents.into_iter().collect(),
    ))
}

struct Parser<'a> {
    source: &'a str,
    nodes: BTreeMap<String, DirectoryDeclaration>,
    collision_keys: BTreeSet<String>,
    parents: BTreeSet<String>,
}

impl Parser<'_> {
    fn insert_path(
        &mut self,
        path: &str,
        explicit: Option<(u32, &str)>,
        compare: bool,
    ) -> Result<(), String> {
        let mut prefix = String::new();
        for part in path.split('/') {
            let parent = prefix.clone();
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            if compare {
                self.parents.insert(parent);
                if self.parents.len() > 64 {
                    return Err("目录声明超过 64 个比较父层，整块不可用".to_owned());
                }
            }
            let source = explicit
                .filter(|_| prefix == path)
                .map(|(line, _)| markdown::location(self.source, line));
            let label = explicit
                .filter(|_| prefix == path)
                .map_or(
                    part,
                    |(_, label)| if label.is_empty() { part } else { label },
                );
            if let Some(existing) = self.nodes.get_mut(&prefix) {
                if source.is_some() && existing.source.is_some() {
                    return Err("同一目录重复声明，覆盖范围含糊".to_owned());
                }
                if source.is_some() {
                    existing.source = source;
                    existing.label = label.to_owned();
                }
            } else {
                // 必须在逐段扩展时拒绝；不能先分配整条深路径的全部前缀再检查。
                if self.nodes.len() >= 256 {
                    return Err("目录声明超过 256 个目录，整块不可用".to_owned());
                }
                if !self.collision_keys.insert(prefix.to_lowercase()) {
                    return Err("目录声明存在大小写路径碰撞".to_owned());
                }
                self.nodes.insert(
                    prefix.clone(),
                    DirectoryDeclaration {
                        path: prefix.clone(),
                        label: label.to_owned(),
                        source,
                    },
                );
            }
        }
        Ok(())
    }
}

fn split_label(text: &str) -> (&str, &str) {
    text.split_once(" #")
        .map_or((text.trim(), ""), |(path, label)| {
            (path.trim(), label.trim())
        })
}

fn branch(mut raw: &str) -> Option<(usize, &str)> {
    let mut depth = 1;
    loop {
        if let Some(tail) = raw
            .strip_prefix("│   ")
            .or_else(|| raw.strip_prefix("    "))
        {
            depth += 1;
            raw = tail;
        } else {
            return raw
                .strip_prefix("├── ")
                .or_else(|| raw.strip_prefix("└── "))
                .map(|entry| (depth, entry));
        }
    }
}

fn declaration_path(path: &str) -> bool {
    repository_path(path) && unambiguous_path(path)
}

fn hidden_file(path: &str) -> bool {
    VaultPath::parse(path).is_ok()
        && unambiguous_path(path)
        && path.split('/').any(|part| part.starts_with('.'))
}

fn unambiguous_path(path: &str) -> bool {
    !path.contains("...")
        && !path.contains('…')
        && !path.contains(['*', '?', '[', ']', '`', '|'])
        && !path.split('/').any(|part| part.trim() != part)
}

pub(crate) fn repository_path(path: &str) -> bool {
    tree_validate::visible_path(path) && !path.chars().any(|value| {
        matches!(value, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
    })
}
