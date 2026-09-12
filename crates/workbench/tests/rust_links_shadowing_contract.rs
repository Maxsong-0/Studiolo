//! 名字阻断覆盖参数和模式的整个保守作用域，不追踪赋值或执行顺序。

#[path = "support/rust_links.rs"]
mod support;

use support::*;

#[test]
fn parameter_and_closure_patterns_block_without_needing_a_body_block() {
    for caller in [
        "fn run(helper: fn()){helper();}",
        "fn run((helper, _): (fn(),u8)){helper();}",
        "fn run(){let c=|helper|helper();}",
        "fn run(){let c=|helper: fn()|helper();}",
        "fn run(){let c=|(helper,_)|helper();}",
    ] {
        all_reason(
            &analyze(&format!("fn helper(){{}} {caller}")),
            "shadowed_name",
        );
    }
}

#[test]
fn let_bindings_ignore_statement_order_and_cover_destructuring() {
    for pattern in [
        "helper",
        "mut helper",
        "ref helper",
        "(helper,_)",
        "[helper]",
        "S { helper }",
        "S { value: helper }",
        "helper @ _",
    ] {
        let source =
            format!("fn helper(){{}} fn run(){{helper(); let {pattern}=value; helper();}}");
        all_reason(&analyze(&source), "shadowed_name");
    }
}

#[test]
fn conditional_loop_and_match_bindings_cover_their_expression_children() {
    for body in [
        "if let Some(helper)=value {helper();}",
        "while let Some(helper)=value {helper();}",
        "for helper in values {helper();}",
        "match value {Some(helper)=>helper(),_=>0}",
        "if let Some(helper)=value && helper() {helper();}",
    ] {
        let analysis = analyze(&format!("fn helper(){{}} fn run(){{{body}}}"));
        all_reason(&analysis, "shadowed_name");
    }
}

#[test]
fn match_guard_let_chains_block_the_arm_but_not_the_next_arm() {
    let analysis = analyze(
        "fn helper(){} fn run(){ match value { _ if let Some(helper)=value && helper() => helper(), _ => helper() } }",
    );
    let links = named(&analysis, "helper");
    assert_eq!(links.len(), 3);
    reason(links[0], "shadowed_name");
    reason(links[1], "shadowed_name");
    assert_eq!(links[2].status, "linked");
}

#[test]
fn match_guard_and_binding_rhs_calls_are_not_collected_as_pattern_names() {
    let analysis = analyze(
        "fn helper(){} fn run(){ let x=helper(); if let Some(y)=helper(){helper();} match value { Some(z) if helper()=>helper(), _=>helper() } }",
    );
    assert_eq!(analysis.links.associations.len(), 6);
    assert!(
        analysis
            .links
            .associations
            .iter()
            .all(|link| link.status == "linked")
    );
}

#[test]
fn generic_names_and_outer_conservative_bindings_are_not_skipped_for_nested_functions() {
    for source in [
        "fn helper(){} fn run<helper>(){helper();}",
        "fn helper(){} fn run<const helper:usize>(){helper();}",
        "fn helper(){} fn run(){let helper=0; fn nested(){helper();}}",
        "fn helper(){} fn run<helper>(){fn nested(){helper();}}",
    ] {
        all_reason(&analyze(source), "shadowed_name");
    }
}

#[test]
fn non_function_items_and_bodyless_declarations_stop_outer_lookup() {
    for declaration in [
        "const helper:u8=0;",
        "static helper:u8=0;",
        "struct helper;",
        "struct helper(u8);",
        "struct helper { value:u8 }",
        "enum helper { Value }",
        "type helper=u8;",
        "mod helper {}",
        "fn helper();",
    ] {
        let source = format!("fn helper(){{}} fn run(){{{declaration} helper();}}");
        all_reason(&analyze(&source), "shadowed_name");
    }
}

#[test]
fn body_item_scope_can_provide_a_nearer_function_than_a_parameter_scope() {
    let analysis = analyze(
        "fn run(helper:fn()){fn helper(){} helper(); let c=|helper|{fn helper(){} helper();};}",
    );
    assert_eq!(analysis.links.associations.len(), 2);
    assert!(
        analysis
            .links
            .associations
            .iter()
            .all(|link| link.status == "linked")
    );
}
