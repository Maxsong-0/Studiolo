//! 预算必须原子拒绝；零调用结果不能用截断实现。

#[path = "support/rust.rs"]
mod support;

use studiolo_workbench::{RustSourceErrorKind::Unsupported, derive_rust_call_sites};
use support::*;

#[test]
fn context_budget_includes_every_file_root_and_allows_exactly_256() {
    let source = (0..255)
        .map(|index| format!("fn f{index}(){{}}\n"))
        .collect::<String>();
    let graph = derive_rust_call_sites(&[input("a.rs", &source)]).unwrap();
    assert_eq!(graph.contexts.len(), 256);
    error(
        derive_rust_call_sites(&[input("a.rs", &source), input("b.rs", "")]),
        Unsupported,
    );
    error(
        derive_rust_call_sites(&[input("a.rs", &format!("{source}fn extra(){{}}"))]),
        Unsupported,
    );
}

#[test]
fn call_budget_is_inclusive_and_shared_across_files() {
    let source = format!("fn f(){{ {} }}", "call();".repeat(1024));
    assert_eq!(
        derive_rust_call_sites(&[input("a.rs", &source)])
            .unwrap()
            .calls
            .len(),
        1024
    );
    error(
        derive_rust_call_sites(&[input("a.rs", &source), input("b.rs", "fn b(){ extra(); }")]),
        Unsupported,
    );
}

#[test]
fn tree_nodes_are_counted_across_files_even_when_there_are_no_calls() {
    let source = format!("fn f(){{ {} }}", "0;".repeat(700));
    assert!(
        derive_rust_call_sites(&[input("a.rs", &source)])
            .unwrap()
            .calls
            .is_empty()
    );
    let inputs = (0..16)
        .map(|index| input(&format!("f{index}.rs"), &source))
        .collect::<Vec<_>>();
    error(derive_rust_call_sites(&inputs), Unsupported);
}

#[test]
fn excessive_depth_in_tokens_is_rejected_before_skipping_macro_body() {
    let shallow = format!("show!({}0{});", "(".repeat(20), ")".repeat(20));
    assert_eq!(
        derive_rust_call_sites(&[input("a.rs", &shallow)])
            .unwrap()
            .calls
            .len(),
        1
    );
    let deep = format!("show!({}0{});", "(".repeat(65), ")".repeat(65));
    error(derive_rust_call_sites(&[input("a.rs", &deep)]), Unsupported);
}

#[test]
fn name_byte_budget_applies_to_contexts_and_combined_identifier_paths() {
    let exact = "a".repeat(256);
    assert!(derive_rust_call_sites(&[input("a.rs", &format!("fn {exact}(){{}}"))]).is_ok());
    error(
        derive_rust_call_sites(&[input("a.rs", &format!("fn {}(){{}}", "a".repeat(257)))]),
        Unsupported,
    );
    error(
        derive_rust_call_sites(&[input(
            "a.rs",
            &format!("fn f(){{ {}::{}(); }}", "a".repeat(128), "b".repeat(127)),
        )]),
        Unsupported,
    );
    assert!(
        derive_rust_call_sites(&[input("a.rs", &format!("fn {}(){{}}", "界".repeat(85)))]).is_ok()
    );
    error(
        derive_rust_call_sites(&[input("a.rs", &format!("fn {}(){{}}", "界".repeat(86)))]),
        Unsupported,
    );
}

#[test]
fn generator_blocks_fail_explicitly_instead_of_misattributing_deferred_calls() {
    error(
        derive_rust_call_sites(&[input(
            "a.rs",
            "fn f(){ let g = gen { yield callback(); }; }",
        )]),
        Unsupported,
    );
}
