//! 仅收集名字阻断；不解析导入、类型、模式值或宏展开。

use tree_sitter::Node;
use unicode_normalization::UnicodeNormalization;

use crate::{RustSourceError, rust_scopes::Scope, rust_walk};

pub(crate) fn key(node: Node<'_>, source: &str) -> Option<String> {
    if !matches!(
        node.kind(),
        "identifier"
            | "type_identifier"
            | "field_identifier"
            | "shorthand_field_identifier"
            | "self"
            | "super"
            | "crate"
    ) {
        return None;
    }
    let text = source.get(node.byte_range())?;
    Some(text.strip_prefix("r#").unwrap_or(text).nfc().collect())
}

pub(crate) fn named_item(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_signature_item"
            | "mod_item"
            | "struct_item"
            | "union_item"
            | "enum_item"
            | "enum_variant"
            | "trait_item"
            | "const_item"
            | "static_item"
            | "type_item"
            | "associated_type"
    )
}

pub(crate) fn collect(
    node: Node<'_>,
    source: &str,
    scope: &mut Scope,
) -> Result<(), RustSourceError> {
    match node.kind() {
        "use_declaration" => names(node, source, scope, true),
        "parameters" => {
            let mut cursor = node.walk();
            for parameter in node.named_children(&mut cursor) {
                if !matches!(
                    parameter.kind(),
                    "parameter"
                        | "self_parameter"
                        | "variadic_parameter"
                        | "attribute_item"
                        | "line_comment"
                        | "block_comment"
                ) {
                    scope.opaque = true;
                }
            }
            Ok(())
        }
        "let_declaration" | "let_condition" | "for_expression" | "match_arm" | "parameter"
        | "variadic_parameter" => {
            if let Some(pattern) = node.child_by_field_name("pattern") {
                names(pattern, source, scope, false)?;
            } else if node.kind() != "variadic_parameter" {
                scope.opaque = true;
            }
            Ok(())
        }
        "closure_parameters" => {
            let mut cursor = node.walk();
            for pattern in node.named_children(&mut cursor) {
                if !matches!(
                    pattern.kind(),
                    "parameter" | "line_comment" | "block_comment"
                ) {
                    names(pattern, source, scope, false)?;
                }
            }
            Ok(())
        }
        "self_parameter" => {
            scope.block("self".to_owned());
            Ok(())
        }
        "type_parameter" | "const_parameter" | "lifetime_parameter" => {
            if let Some(name) = node.child_by_field_name("name") {
                names(name, source, scope, false)?;
            } else {
                scope.opaque = true;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn names(
    root: Node<'_>,
    source: &str,
    scope: &mut Scope,
    importing: bool,
) -> Result<(), RustSourceError> {
    // match_pattern 的 guard 是表达式；其 let 条件由主遍历单独登记。
    let guard = root.child_by_field_name("condition").map(|node| node.id());
    rust_walk::walk(root, |node, entering| {
        if !entering {
            return Ok(false);
        }
        if Some(node.id()) == guard {
            return Ok(false);
        }
        if matches!(
            node.kind(),
            "line_comment" | "block_comment" | "macro_definition"
        ) {
            return Ok(false);
        }
        if matches!(
            node.kind(),
            "macro_invocation"
                | "metavariable"
                | "const_block"
                | "closure_expression"
                | "attribute_item"
                | "inner_attribute_item"
        ) || importing && node.kind() == "use_wildcard"
        {
            scope.opaque = true;
            return Ok(false);
        }
        if let Some(name) = key(node, source) {
            scope.block(name);
        }
        Ok(true)
    })
}
