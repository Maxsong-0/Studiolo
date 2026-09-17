//! 标识路径、表达式 callee 与未展开 token 的语法边界。

#[path = "support/rust.rs"]
mod support;

use studiolo_workbench::derive_rust_call_sites;
use support::*;

#[test]
fn raw_absolute_and_generic_paths_drop_type_arguments_but_not_identifiers() {
    let source = "fn r#match(){ ::dep::r#type::<u8>(); Type::<u8>::new(); self::run(); super::run(); crate::run(); <Type>::run(); }";
    let graph = derive_rust_call_sites(&[input("a.rs", source)]).unwrap();
    assert_eq!(graph.contexts[1].name.as_deref(), Some("r#match"));
    for name in [
        "::dep::r#type",
        "Type::new",
        "self::run",
        "super::run",
        "crate::run",
    ] {
        assert!(
            graph
                .calls
                .iter()
                .any(|call| call.kind == "path" && call.name.as_deref() == Some(name)),
            "{name}"
        );
    }
    assert_eq!(
        graph
            .calls
            .iter()
            .filter(|call| call.kind == "expression")
            .count(),
        1
    );
    assert!(!format!("{graph:?}").contains("u8"));
}

#[test]
fn computed_and_field_callables_are_not_named_as_resolved_functions() {
    let graph = derive_rust_call_sites(&[input(
        "a.rs",
        "fn f(){ callbacks[0](); (obj.callback)(); obj.0(); obj.call(); }",
    )])
    .unwrap();
    assert_eq!(
        graph
            .calls
            .iter()
            .filter(|call| call.kind == "expression")
            .count(),
        3
    );
    assert_eq!(
        graph
            .calls
            .iter()
            .filter(|call| call.kind == "method" && call.name.as_deref() == Some("call"))
            .count(),
        1
    );
    references(&graph);
}

#[test]
fn macro_definitions_attributes_comments_and_strings_do_not_produce_token_calls() {
    let source = r###"#![doc = concat!("PRIVATE_VALUE", "hidden()")]
#[/*说明*/ cfg_attr(any(), tool::attr(fake()))]
mod absent;
macro_rules! make { () => { fn generated() { hidden(); } }; }
fn f() {
    // comment_call();
    let text = r##"string_call();"##;
    include!("PRIVATE_VALUE");
    actual!({ tokens_call(); nested!(); });
}
"###;
    let graph = derive_rust_call_sites(&[input("a.rs", source)]).unwrap();
    assert_eq!(graph.calls.len(), 2);
    assert!(graph.calls.iter().all(|call| call.kind == "macro"));
    assert_eq!(graph.files[0].attributes, 2);
    assert_eq!(graph.files[0].conditional_attributes, 1);
    assert_eq!(graph.files[0].unexpanded_macros, 2);
    assert_eq!(graph.files[0].macro_definitions, 1);
    assert_eq!(graph.files[0].external_modules, 1);
    assert!(!format!("{graph:?}").contains("PRIVATE_VALUE"));
}

#[test]
fn cfg_is_not_evaluated_and_same_named_methods_are_separate_sites() {
    let source =
        "#[cfg(any())]\nfn disabled(){ a.send(); }\n#[cfg(all())]\nfn enabled(){ b.send(); }";
    let graph = derive_rust_call_sites(&[input("a.rs", source)]).unwrap();
    assert_eq!(graph.calls.len(), 2);
    assert_eq!(graph.files[0].conditional_attributes, 2);
    assert_ne!(graph.calls[0].id, graph.calls[1].id);
    assert_ne!(graph.calls[0].context, graph.calls[1].context);
    assert!(
        graph
            .calls
            .iter()
            .all(|call| call.name.as_deref() == Some("send"))
    );
}
