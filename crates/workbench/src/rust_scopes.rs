//! 全量作用域索引；所有名字和不透明标记收齐后才允许目标查找。

use std::collections::BTreeMap;

use tree_sitter::{Node, Tree};

use crate::{RustSourceError, RustSourceInput, rust_bindings, rust_extract, rust_walk};

#[derive(Default)]
pub(crate) struct Name {
    pub blocker: bool,
    pub candidates: Vec<Candidate>,
}

pub(crate) struct Candidate {
    pub target: String,
    pub generic: bool,
}

#[derive(Default)]
pub(crate) struct Scope {
    pub parent: Option<usize>,
    pub boundary: bool,
    pub function: bool,
    pub forbidden: bool,
    pub opaque: bool,
    pub names: BTreeMap<String, Name>,
}

impl Scope {
    pub(crate) fn block(&mut self, key: String) {
        self.names.entry(key).or_default().blocker = true;
    }
}

pub(crate) struct Call {
    pub scope: usize,
    pub key: Option<String>,
}

pub(crate) struct Index {
    pub scopes: Vec<Scope>,
    pub calls: BTreeMap<String, Call>,
}

pub(crate) fn build(
    tree: &Tree,
    input: &RustSourceInput,
    file: usize,
) -> Result<Index, RustSourceError> {
    let mut builder = Builder {
        source: &input.content,
        file,
        index: Index {
            scopes: Vec::new(),
            calls: BTreeMap::new(),
        },
        stack: Vec::new(),
    };
    rust_walk::walk(tree.root_node(), |node, entering| {
        builder.visit(node, entering)
    })?;
    Ok(builder.index)
}

struct Builder<'a> {
    source: &'a str,
    file: usize,
    index: Index,
    stack: Vec<usize>,
}

impl Builder<'_> {
    fn visit(&mut self, node: Node<'_>, entering: bool) -> Result<bool, RustSourceError> {
        if !entering {
            self.stack.pop();
            return Ok(false);
        }
        let parent = self.stack.last().copied();
        let free = rust_extract::context_kind(node) == Some("function");
        if let Some(parent) = parent {
            self.declaration(node, parent, free);
        }
        let scope = if creates_scope(node) {
            let ancestor = parent.map(|id| &self.index.scopes[id]);
            let scope = Scope {
                parent,
                boundary: matches!(node.kind(), "source_file" | "mod_item"),
                function: free || ancestor.is_some_and(|scope| scope.function),
                forbidden: forbidden(node) || ancestor.is_some_and(|scope| scope.forbidden),
                ..Scope::default()
            };
            let id = self.index.scopes.len();
            self.index.scopes.push(scope);
            id
        } else {
            parent.ok_or_else(|| RustSourceError::invalid("Rust 关联缺少文件根作用域。"))?
        };
        self.stack.push(scope);
        if matches!(
            node.kind(),
            "macro_definition" | "line_comment" | "block_comment"
        ) {
            return Ok(false);
        }
        if matches!(node.kind(), "call_expression" | "macro_invocation") {
            let key = node
                .child_by_field_name("function")
                .filter(|callee| callee.kind() == "identifier")
                .and_then(|callee| rust_bindings::key(callee, self.source));
            self.index
                .calls
                .insert(self.id("call", node), Call { scope, key });
        }
        if matches!(
            node.kind(),
            "attribute_item" | "inner_attribute_item" | "macro_invocation"
        ) {
            self.index.scopes[scope].opaque = true;
            return Ok(false);
        }
        // extern block 不建立额外名字容器，直接使其外层 scope 不透明。
        if matches!(
            node.kind(),
            "foreign_mod_item" | "extern_crate_declaration" | "extern_modifier" | "metavariable"
        ) {
            self.index.scopes[scope].opaque = true;
        }
        rust_bindings::collect(node, self.source, &mut self.index.scopes[scope])?;
        Ok(true)
    }

    fn declaration(&mut self, node: Node<'_>, parent: usize, free: bool) {
        if !free && !rust_bindings::named_item(node) {
            return;
        }
        let name = node
            .child_by_field_name("name")
            .and_then(|name| rust_bindings::key(name, self.source));
        let Some(name) = name else {
            self.index.scopes[parent].opaque = true;
            return;
        };
        if free {
            let candidate = Candidate {
                target: self.id("context", node),
                generic: node.child_by_field_name("type_parameters").is_some(),
            };
            self.index.scopes[parent]
                .names
                .entry(name)
                .or_default()
                .candidates
                .push(candidate);
        } else {
            self.index.scopes[parent].block(name);
        }
    }

    fn id(&self, kind: &str, node: Node<'_>) -> String {
        format!(
            "f{}:{kind}:{}:{}",
            self.file,
            node.start_byte(),
            node.end_byte()
        )
    }
}

fn creates_scope(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "source_file"
            | "block"
            | "function_item"
            | "function_signature_item"
            | "closure_expression"
            | "async_block"
            | "const_block"
            | "impl_item"
            | "trait_item"
            | "const_item"
            | "static_item"
            | "struct_item"
            | "union_item"
            | "enum_item"
            | "type_item"
            | "if_expression"
            | "while_expression"
            | "for_expression"
            | "match_arm"
    ) || node.kind() == "mod_item" && node.child_by_field_name("body").is_some()
}

fn forbidden(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "impl_item" | "trait_item" | "const_item" | "const_block" | "static_item"
    )
}
