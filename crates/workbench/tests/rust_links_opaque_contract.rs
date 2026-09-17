//! 导入和不透明语法按完整作用域生效，不依赖遇见节点的先后。

#[path = "support/rust_links.rs"]
mod support;

use support::*;

#[test]
fn unrelated_imports_do_not_disable_file_and_aliases_block_both_spellings() {
    let good = analyze("use external::Other; fn helper(){} fn run(){helper();}");
    assert_eq!(good.links.associations[0].status, "linked");
    let imported = analyze(
        "use external::{helper as alias}; fn helper(){} fn alias(){} fn run(){helper();alias();}",
    );
    all_reason(&imported, "shadowed_name");
}

#[test]
fn scoped_glob_import_is_opaque_before_and_after_its_source_position() {
    let analysis = analyze(
        "fn helper(){} fn run(){helper(); {helper(); use external::*; helper();} helper();}",
    );
    let links = named(&analysis, "helper");
    assert_eq!(links.len(), 4);
    assert_eq!(links[0].status, "linked");
    reason(links[1], "opaque_scope");
    reason(links[2], "opaque_scope");
    assert_eq!(links[3].status, "linked");
}

#[test]
fn sibling_attributes_make_the_whole_scope_opaque_including_prior_callers() {
    for attribute in ["#[cfg(any())]", "#[allow(dead_code)]", "#[tool::transform]"] {
        let analysis = analyze(&format!(
            "fn run(){{helper();}} {attribute} fn helper(){{}}"
        ));
        all_reason(&analysis, "opaque_scope");
    }
    let inner = analyze("fn helper(){} fn run(){#![allow(unused)] helper();}");
    all_reason(&inner, "opaque_scope");
}

#[test]
fn macro_invocations_only_make_their_own_scope_and_descendants_opaque() {
    let analysis = analyze(
        "fn helper(){make!();} fn run(){helper(); {helper(); make!(); helper();} helper();}",
    );
    let links = named(&analysis, "helper");
    assert_eq!(links.len(), 4);
    assert_eq!(links[0].status, "linked");
    reason(links[1], "opaque_scope");
    reason(links[2], "opaque_scope");
    assert_eq!(links[3].status, "linked");
    for link in named(&analysis, "make") {
        reason(link, "callee_form");
    }
}

#[test]
fn macro_definitions_and_documentation_comments_do_not_inject_values_or_hazards() {
    let source = "/// #[cfg(any())] use x::*;\nmacro_rules! make { () => { use x::*; fn helper(){} }; }\nfn helper(){}\nfn run(){helper();}";
    let analysis = analyze(source);
    assert_eq!(analysis.links.associations.len(), 1);
    assert_eq!(analysis.links.associations[0].status, "linked");
}

#[test]
fn extern_blocks_poison_the_outer_scope_instead_of_hiding_declarations() {
    for declaration in ["unsafe extern \"C\" {fn helper();}", "extern crate remote;"] {
        let source = format!("fn helper(){{}} fn run(){{helper();}} {declaration}");
        all_reason(&analyze(&source), "opaque_scope");
    }
}

#[test]
fn reason_priority_is_callee_then_context_then_opacity_then_name() {
    let callee = analyze("#[allow(unused)] fn helper(){} impl A {fn run(){helper::<u8>();}}");
    all_reason(&callee, "callee_form");
    let context = analyze("#[allow(unused)] fn helper(){} impl A {fn run(){helper();}}");
    all_reason(&context, "context");
    let opaque =
        analyze("fn helper(){} fn run(){let helper=0; helper(); #[allow(unused)] let x=0;}");
    all_reason(&opaque, "opaque_scope");
}

#[test]
fn a_macro_in_a_binding_pattern_cannot_hide_possible_shadowing() {
    let analysis = analyze("fn helper(){} fn run(){let bind!()=value; helper();}");
    reason(named(&analysis, "bind")[0], "callee_form");
    reason(named(&analysis, "helper")[0], "opaque_scope");
}
