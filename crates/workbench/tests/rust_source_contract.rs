//! 来源范围、字节列、确定排序及无效语法的原子拒绝。

#[path = "support/rust.rs"]
mod support;

use studiolo_workbench::{RustSourceErrorKind::Invalid, derive_rust_call_sites};
use support::*;

#[test]
fn nested_calls_sort_by_start_then_end_and_do_not_depend_on_input_order() {
    let source = "fn f(){ factory()(); }";
    let inputs = [input("z.rs", "fn z(){ zed(); }"), input("a.rs", source)];
    let graph = derive_rust_call_sites(&inputs).unwrap();
    let reversed = derive_rust_call_sites(&[inputs[1].clone(), inputs[0].clone()]).unwrap();
    assert_eq!(graph, reversed);
    assert_eq!(graph.calls[0].id, "f0:call:8:17");
    assert_eq!(graph.calls[1].id, "f0:call:8:19");
    assert_eq!(graph.calls[0].source.column, 9);
    assert_eq!(graph.files[0].path, "a.rs");
    references(&graph);
}

#[test]
fn unicode_crlf_and_shebang_sources_retain_utf8_byte_columns() {
    let source =
        "#!/usr/bin/env rust-script\r\nfn 入口() {\r\n    let 文 = \"不输出\"; 调用();\r\n}\r\n";
    let graph = derive_rust_call_sites(&[input("源码.rs", source)]).unwrap();
    assert_eq!(graph.contexts[0].source.line, 1);
    assert_eq!(graph.contexts[0].source.column, 1);
    assert_eq!(graph.contexts[1].source.line, 2);
    assert_eq!(graph.calls[0].source.line, 3);
    assert_eq!(graph.calls[0].source.column, 28);
    assert_eq!(graph.calls[0].name.as_deref(), Some("调用"));
    assert!(!format!("{graph:?}").contains("不输出"));
}

#[test]
fn malformed_or_missing_syntax_rejects_the_entire_batch_without_parser_text() {
    for content in [
        "fn PRIVATE_VALUE( {",
        "fn f(){ let x = 1 }",
        "fn f(){ a( ; }",
    ] {
        error(
            derive_rust_call_sites(&[
                input("a.rs", "fn good(){ good(); }"),
                input("z.rs", content),
            ]),
            Invalid,
        );
    }
}
