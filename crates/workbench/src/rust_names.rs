//! 只从标识节点提取名称；不拼接接收者、参数或限定类型正文。

use tree_sitter::Node;

use crate::{RustSourceError, rust_paths::invisible};

pub(crate) fn identifier(node: Node<'_>, source: &str) -> Result<Option<String>, RustSourceError> {
    if !matches!(
        node.kind(),
        "identifier" | "type_identifier" | "field_identifier" | "self" | "super" | "crate"
    ) {
        return Ok(None);
    }
    let value = &source[node.byte_range()];
    if value.is_empty() || value.chars().any(invisible) {
        return Err(RustSourceError::invalid(
            "Rust 输出名称包含空值或不可见字符。",
        ));
    }
    if value.len() > 256 {
        return Err(RustSourceError::unsupported("Rust 输出名称超过 256 字节。"));
    }
    Ok(Some(value.to_owned()))
}

pub(crate) fn path(mut node: Node<'_>, source: &str) -> Result<Option<String>, RustSourceError> {
    let mut parts = Vec::new();
    let mut bytes = 0;
    let mut absolute = false;
    loop {
        match node.kind() {
            "generic_function" | "generic_type" | "generic_type_with_turbofish" => {
                let field = if node.kind() == "generic_function" {
                    "function"
                } else {
                    "type"
                };
                let Some(inner) = node.child_by_field_name(field) else {
                    return Ok(None);
                };
                node = inner;
            }
            "scoped_identifier" | "scoped_type_identifier" => {
                let Some(name) = node.child_by_field_name("name") else {
                    return Ok(None);
                };
                let Some(name) = identifier(name, source)? else {
                    return Ok(None);
                };
                bytes += name.len() + 2;
                if bytes > 258 {
                    return Err(RustSourceError::unsupported("Rust 输出名称超过 256 字节。"));
                }
                parts.push(name);
                let Some(parent) = node.child_by_field_name("path") else {
                    absolute = true;
                    break;
                };
                node = parent;
            }
            _ => {
                let Some(name) = identifier(node, source)? else {
                    return Ok(None);
                };
                parts.push(name);
                break;
            }
        }
    }
    parts.reverse();
    let value = format!("{}{}", if absolute { "::" } else { "" }, parts.join("::"));
    if value.len() > 256 {
        return Err(RustSourceError::unsupported("Rust 输出名称超过 256 字节。"));
    }
    Ok(Some(value))
}

pub(crate) fn callee(
    mut node: Node<'_>,
    source: &str,
) -> Result<(&'static str, Option<String>), RustSourceError> {
    if node.kind() == "generic_function" {
        node = node
            .child_by_field_name("function")
            .ok_or_else(|| RustSourceError::invalid("Rust 调用缺少语法函数字段。"))?;
    }
    if node.kind() == "field_expression" {
        if let Some(field) = node.child_by_field_name("field")
            && let Some(name) = identifier(field, source)?
        {
            return Ok(("method", Some(name)));
        }
    } else if let Some(name) = path(node, source)? {
        return Ok(("path", Some(name)));
    }
    Ok(("expression", None))
}
