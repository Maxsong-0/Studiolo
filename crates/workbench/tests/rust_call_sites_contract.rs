//! 词法包含和语法类别保留原文证据，不伪造目标、运行顺序或宏展开。

#[path = "support/rust.rs"]
mod support;

use studiolo_workbench::derive_rust_call_sites;
use support::*;

#[test]
fn paths_methods_expressions_and_macro_invocations_have_distinct_evidence() {
    let source = r##"#![allow(dead_code)]
#[cfg(any())]
mod 外 {
    pub fn execute() {
        dep::r#type::<u8>(secret());
        worker.send::<u8>("PRIVATE_VALUE");
        factory()();
        (callback)();
        <Thing as Trait>::run();
        name!(hidden());
    }
}
mod external;
macro_rules! helper { () => { hidden(); }; }
"##;
    let graph = derive_rust_call_sites(&[input("src/lib.rs", source)]).unwrap();
    assert_eq!(graph.calls.len(), 8);
    let names = graph
        .calls
        .iter()
        .map(|call| (call.kind.as_str(), call.name.as_deref()))
        .collect::<Vec<_>>();
    for expected in [
        ("path", Some("dep::r#type")),
        ("path", Some("secret")),
        ("path", Some("factory")),
        ("method", Some("send")),
        ("macro", Some("name")),
    ] {
        assert!(names.contains(&expected), "{names:?}");
    }
    assert_eq!(
        names.iter().filter(|item| item.0 == "expression").count(),
        3
    );
    let coverage = &graph.files[0];
    assert_eq!(coverage.attributes, 2);
    assert_eq!(coverage.conditional_attributes, 1);
    assert_eq!(coverage.unexpanded_macros, 1);
    assert_eq!(coverage.macro_definitions, 1);
    assert_eq!(coverage.external_modules, 1);
    let output = format!("{graph:?}");
    for excluded in ["PRIVATE_VALUE", "hidden", "worker", "Thing", "Trait"] {
        assert!(!output.contains(excluded), "{excluded}");
    }
    references(&graph);
}

#[test]
fn delayed_and_nested_definitions_have_their_nearest_lexical_context() {
    let source = r#"mod outer {
trait T { fn declaration(); fn defaulted(&self) { trait_call(); } }
impl Thing {
    fn method(&self) {
        fn nested() { nested_call(); }
        let c = || closure_call();
        let f = async { async_call(); };
        const LOCAL: u32 = const { const_call() };
        static VALUE: u32 = static_call();
        direct_call();
    }
}
}
const _: u32 = top_call();
"#;
    let graph = derive_rust_call_sites(&[input("a.rs", source)]).unwrap();
    assert_eq!(graph.contexts.len(), 13);
    assert!(
        !graph
            .contexts
            .iter()
            .any(|context| context.name.as_deref() == Some("declaration"))
    );
    for (name, kind, context_name) in [
        ("trait_call", "method", Some("defaulted")),
        ("nested_call", "function", Some("nested")),
        ("closure_call", "closure", None),
        ("async_call", "async", None),
        ("const_call", "const", None),
        ("static_call", "static", Some("VALUE")),
        ("direct_call", "method", Some("method")),
        ("top_call", "const", None),
    ] {
        let call = graph
            .calls
            .iter()
            .find(|call| call.name.as_deref() == Some(name))
            .unwrap();
        let context = graph
            .contexts
            .iter()
            .find(|context| context.id == call.context)
            .unwrap();
        assert_eq!(
            (context.kind.as_str(), context.name.as_deref()),
            (kind, context_name)
        );
    }
    references(&graph);
}
