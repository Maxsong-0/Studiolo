//! 完整树先验通过后，按词法进入/退出事件生成扁平结果。

use tree_sitter::{Node, Tree};

use crate::{
    RustCallContext, RustCallGraph, RustCallSite, RustSourceCoverage, RustSourceError,
    RustSourceInput, RustSourceLocation, rust_names, rust_walk,
};

pub(crate) fn extract(
    tree: &Tree,
    input: &RustSourceInput,
    index: usize,
    graph: &mut RustCallGraph,
) -> Result<(), RustSourceError> {
    if graph.contexts.len() >= 256 {
        return Err(RustSourceError::unsupported("Rust 词法上下文超过 256 个。"));
    }
    let root_id = format!("f{index}:root");
    let root = RustCallContext {
        id: root_id.clone(),
        parent: None,
        kind: "file".to_owned(),
        name: None,
        source: RustSourceLocation {
            path: input.path.clone(),
            line: 1,
            column: 1,
        },
    };
    let mut state = Extraction {
        input,
        index,
        root_id,
        graph,
        contexts: vec![(0, 0, root)],
        calls: Vec::new(),
        stack: Vec::new(),
        coverage: RustSourceCoverage {
            path: input.path.clone(),
            bytes: input.content.len() as u32,
            attributes: 0,
            conditional_attributes: 0,
            unexpanded_macros: 0,
            macro_definitions: 0,
            external_modules: 0,
        },
    };
    rust_walk::walk(tree.root_node(), |node, entering| {
        state.visit(node, entering)
    })?;
    state.contexts.sort_by_key(|item| (item.0, item.1));
    state.calls.sort_by_key(|item| (item.0, item.1));
    state
        .graph
        .contexts
        .extend(state.contexts.into_iter().map(|item| item.2));
    state
        .graph
        .calls
        .extend(state.calls.into_iter().map(|item| item.2));
    state.graph.files.push(state.coverage);
    Ok(())
}

struct Extraction<'a> {
    input: &'a RustSourceInput,
    index: usize,
    root_id: String,
    graph: &'a mut RustCallGraph,
    contexts: Vec<(usize, usize, RustCallContext)>,
    calls: Vec<(usize, usize, RustCallSite)>,
    stack: Vec<String>,
    coverage: RustSourceCoverage,
}

impl Extraction<'_> {
    fn visit(&mut self, node: Node<'_>, entering: bool) -> Result<bool, RustSourceError> {
        if !entering {
            self.stack.pop();
            return Ok(false);
        }
        let parent = self.stack.last().unwrap_or(&self.root_id).clone();
        let context = if let Some(kind) = context_kind(node) {
            self.context(node, kind, parent)?
        } else {
            parent
        };
        self.stack.push(context.clone());
        match node.kind() {
            "attribute_item" | "inner_attribute_item" => {
                self.coverage.attributes += 1;
                if conditional_attribute(node, &self.input.content) {
                    self.coverage.conditional_attributes += 1;
                }
                return Ok(false);
            }
            "macro_definition" => {
                self.coverage.macro_definitions += 1;
                return Ok(false);
            }
            "macro_invocation" => {
                let name = node
                    .child_by_field_name("macro")
                    .ok_or_else(|| RustSourceError::invalid("Rust 宏调用缺少标识路径。"))?;
                let name = rust_names::path(name, &self.input.content)?
                    .ok_or_else(|| RustSourceError::invalid("Rust 宏调用标识路径不可提取。"))?;
                self.call(node, context, "macro", Some(name))?;
                self.coverage.unexpanded_macros += 1;
                return Ok(false);
            }
            "call_expression" => {
                let function = node
                    .child_by_field_name("function")
                    .ok_or_else(|| RustSourceError::invalid("Rust 调用缺少语法函数字段。"))?;
                let (kind, name) = rust_names::callee(function, &self.input.content)?;
                self.call(node, context, kind, name)?;
            }
            "mod_item" if node.child_by_field_name("body").is_none() => {
                self.coverage.external_modules += 1;
            }
            _ => {}
        }
        Ok(true)
    }

    fn context(
        &mut self,
        node: Node<'_>,
        kind: &str,
        parent: String,
    ) -> Result<String, RustSourceError> {
        if self.graph.contexts.len() + self.contexts.len() >= 256 {
            return Err(RustSourceError::unsupported("Rust 词法上下文超过 256 个。"));
        }
        let name = if matches!(kind, "impl" | "closure" | "async") || node.kind() == "const_block" {
            None
        } else {
            let name = node
                .child_by_field_name("name")
                .ok_or_else(|| RustSourceError::invalid("Rust 词法定义缺少标识。"))?;
            let name = rust_names::identifier(name, &self.input.content)?
                .ok_or_else(|| RustSourceError::invalid("Rust 词法定义标识不可提取。"))?;
            if kind == "const" && name == "_" {
                None
            } else {
                Some(name)
            }
        };
        let id = self.id("context", node);
        self.contexts.push((
            node.start_byte(),
            node.end_byte(),
            RustCallContext {
                id: id.clone(),
                parent: Some(parent),
                kind: kind.to_owned(),
                name,
                source: self.location(node),
            },
        ));
        Ok(id)
    }

    fn call(
        &mut self,
        node: Node<'_>,
        context: String,
        kind: &str,
        name: Option<String>,
    ) -> Result<(), RustSourceError> {
        if self.graph.calls.len() + self.calls.len() >= 1024 {
            return Err(RustSourceError::unsupported("Rust 调用位置超过 1024 个。"));
        }
        self.calls.push((
            node.start_byte(),
            node.end_byte(),
            RustCallSite {
                id: self.id("call", node),
                context,
                kind: kind.to_owned(),
                name,
                source: self.location(node),
            },
        ));
        Ok(())
    }

    fn id(&self, kind: &str, node: Node<'_>) -> String {
        format!(
            "f{}:{kind}:{}:{}",
            self.index,
            node.start_byte(),
            node.end_byte()
        )
    }

    fn location(&self, node: Node<'_>) -> RustSourceLocation {
        let point = node.start_position();
        RustSourceLocation {
            path: self.input.path.clone(),
            line: point.row as u32 + 1,
            column: point.column as u32 + 1,
        }
    }
}

fn context_kind(node: Node<'_>) -> Option<&'static str> {
    match node.kind() {
        "mod_item" if node.child_by_field_name("body").is_some() => Some("module"),
        "impl_item" => Some("impl"),
        "trait_item" => Some("trait"),
        "function_item" if node.child_by_field_name("body").is_some() => {
            let method = node
                .parent()
                .filter(|parent| parent.kind() == "declaration_list")
                .and_then(|parent| parent.parent())
                .is_some_and(|parent| matches!(parent.kind(), "impl_item" | "trait_item"));
            Some(if method { "method" } else { "function" })
        }
        "closure_expression" => Some("closure"),
        "async_block" => Some("async"),
        "const_item" | "const_block" => Some("const"),
        "static_item" => Some("static"),
        _ => None,
    }
}

fn conditional_attribute(node: Node<'_>, source: &str) -> bool {
    let mut cursor = node.walk();
    let Some(attribute) = node
        .named_children(&mut cursor)
        .find(|child| child.kind() == "attribute")
    else {
        return false;
    };
    let mut cursor = attribute.walk();
    let Some(path) = attribute
        .named_children(&mut cursor)
        .find(|child| !matches!(child.kind(), "line_comment" | "block_comment"))
    else {
        return false;
    };
    // 属性只统计，不输出名称；参数 token 不会进入调用提取。
    let value = &source[path.byte_range()];
    matches!(
        value.strip_prefix("r#").unwrap_or(value),
        "cfg" | "cfg_attr"
    )
}
