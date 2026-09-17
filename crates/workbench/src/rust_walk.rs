//! 每文件独立解析；完整树先验和后续遍历均用游标迭代，不递归。

use std::ops::ControlFlow;

use tree_sitter::{Node, ParseOptions, Parser, Tree};

use crate::RustSourceError;

pub(crate) fn parse(source: &str) -> Result<Tree, RustSourceError> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| RustSourceError::unsupported("内置 Rust 语法版本不可用。"))?;
    let mut progress = Progress::default();
    let mut callback = |_: &tree_sitter::ParseState| progress.advance();
    let bytes = source.as_bytes();
    let tree = parser.parse_with_options(
        &mut |offset, _| bytes.get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut callback)),
    );
    if progress.calls > 256 {
        return Err(RustSourceError::unsupported(
            "Rust 语法解析超过合作式进度预算。",
        ));
    }
    tree.ok_or_else(|| RustSourceError::unsupported("Rust 语法解析超过合作式进度预算。"))
}

#[derive(Default)]
struct Progress {
    calls: usize,
}

impl Progress {
    fn advance(&mut self) -> ControlFlow<()> {
        self.calls += 1;
        if self.calls > 256 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

pub(crate) fn validate(tree: &Tree, total: &mut usize) -> Result<(), RustSourceError> {
    let mut depth = 0;
    walk(tree.root_node(), |node, entering| {
        if !entering {
            depth -= 1;
            return Ok(false);
        }
        depth += 1;
        *total += 1;
        if *total > 32768 || depth > 64 {
            return Err(RustSourceError::unsupported(
                "Rust 语法树超过 32768 节点或 64 层预算。",
            ));
        }
        if node.is_error() || node.is_missing() {
            return Err(RustSourceError::invalid(
                "Rust 源码包含错误或缺失的语法节点。",
            ));
        }
        if node.kind() == "gen_block" {
            return Err(RustSourceError::unsupported(
                "生成器块不在本片词法上下文支持范围内。",
            ));
        }
        Ok(true)
    })
}

/// 每个节点先进入再退出；进入返回 false 仅跳过其孩子，退出仍配对发生。
pub(crate) fn walk<'tree>(
    root: Node<'tree>,
    mut visit: impl FnMut(Node<'tree>, bool) -> Result<bool, RustSourceError>,
) -> Result<(), RustSourceError> {
    let mut cursor = root.walk();
    loop {
        if visit(cursor.node(), true)? && cursor.goto_first_child() {
            continue;
        }
        loop {
            visit(cursor.node(), false)?;
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_cancels_on_the_257th_callback() {
        let mut progress = Progress::default();
        for _ in 0..256 {
            assert_eq!(progress.advance(), ControlFlow::Continue(()));
        }
        assert_eq!(progress.advance(), ControlFlow::Break(()));
    }
}
