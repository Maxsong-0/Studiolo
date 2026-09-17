//! 正向定义和反向引用必须来自最近允许的同文件词法范围。

#[path = "support/rust_links.rs"]
mod support;

use studiolo_workbench::derive_rust_call_analysis;
use support::*;

#[test]
fn forward_references_recursion_and_reverse_references_share_one_definition() {
    let analysis = analyze("fn run(){ helper(); helper(); }\nfn helper(){ helper(); }");
    let target = analysis
        .graph
        .contexts
        .iter()
        .find(|context| context.name.as_deref() == Some("helper"))
        .unwrap();
    assert_eq!(analysis.links.associations.len(), 3);
    assert!(
        analysis
            .links
            .associations
            .iter()
            .all(|link| link.target.as_deref() == Some(target.id.as_str()))
    );
    let callers = analysis
        .graph
        .calls
        .iter()
        .filter(|call| {
            analysis.links.associations.iter().any(|link| {
                link.call_id == call.id && link.target.as_deref() == Some(target.id.as_str())
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(callers.len(), 3);
    assert_eq!(target.source.line, 2);
}

#[test]
fn nearest_block_definition_wins_and_closure_async_keep_ancestor_lookup() {
    let analysis = analyze(
        "fn helper(){} fn run(){ { fn helper(){} helper(); } let c=||helper(); let f=async {helper();}; }",
    );
    let helpers = analysis
        .graph
        .contexts
        .iter()
        .filter(|context| context.name.as_deref() == Some("helper"))
        .collect::<Vec<_>>();
    let links = named(&analysis, "helper");
    assert_eq!(helpers.len(), 2);
    assert_eq!(links[0].target.as_deref(), Some(helpers[1].id.as_str()));
    assert_eq!(links[1].target.as_deref(), Some(helpers[0].id.as_str()));
    assert_eq!(links[2].target.as_deref(), Some(helpers[0].id.as_str()));
}

#[test]
fn nested_functions_can_find_block_items_but_modules_do_not_inherit_parent_names() {
    let analysis = analyze(
        "fn helper(){} fn run(){ fn nested(){helper();} nested(); mod child { fn call(){helper();} } }",
    );
    let links = named(&analysis, "helper");
    assert_eq!(links[0].status, "linked");
    reason(links[1], "not_found");
    assert_eq!(named(&analysis, "nested")[0].status, "linked");
}

#[test]
fn inline_module_uses_its_own_definition_and_other_files_never_supply_targets() {
    let analysis = derive_rust_call_analysis(&[
        input(
            "a.rs",
            "mod child { fn local(){} fn run(){local(); missing();} }",
        ),
        input("b.rs", "fn missing(){} fn run(){missing();}"),
    ])
    .unwrap();
    references(&analysis);
    assert_eq!(named(&analysis, "local")[0].status, "linked");
    let missing = named(&analysis, "missing");
    reason(missing[0], "not_found");
    assert_eq!(missing[1].status, "linked");
}

#[test]
fn duplicate_candidates_and_generic_definitions_have_distinct_reasons() {
    let ambiguous = analyze("fn helper(){} fn helper<T>(){} fn run(){helper();}");
    all_reason(&ambiguous, "ambiguous_name");
    let generic = analyze("fn helper<T>(){} fn run(){helper();}");
    all_reason(&generic, "generic_target");
    let shadowed = analyze("fn run(){fn helper(){} fn helper<T>(){} let helper=0; helper();}");
    all_reason(&shadowed, "shadowed_name");
}

#[test]
fn only_direct_identifier_calls_are_eligible_even_when_a_name_is_visible() {
    let analysis = analyze(
        "fn helper(){} fn run(){helper::<u8>(); crate::helper(); (helper)(); obj.helper(); helper!();}",
    );
    all_reason(&analysis, "callee_form");
}

#[test]
fn forbidden_ancestors_and_file_level_type_expressions_are_not_function_callers() {
    let analysis = analyze(
        "fn helper(){} impl A { fn run(){helper(); let c=||helper(); fn inner(){helper();}} } trait T {fn run(){helper();}} const C:u8=helper(); static S:u8=helper(); type A=[u8;helper()]; enum E { V=helper() } fn run(){const {helper()};}",
    );
    all_reason(&analysis, "context");
}
