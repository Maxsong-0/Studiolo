//! 内部名字身份去 raw 前缀并做 NFC，显示内容仍保留原文。

#[path = "support/rust_links.rs"]
mod support;

use support::*;

#[test]
fn raw_and_ordinary_identifiers_share_identity_without_rewriting_display() {
    let analysis = analyze("fn r#helper(){} fn run(){helper(); r#helper();}");
    let target = analysis
        .graph
        .contexts
        .iter()
        .find(|context| context.name.as_deref() == Some("r#helper"))
        .unwrap();
    assert!(
        analysis
            .links
            .associations
            .iter()
            .all(|link| link.target.as_deref() == Some(target.id.as_str()))
    );
    assert_eq!(analysis.graph.calls[0].name.as_deref(), Some("helper"));
    assert_eq!(analysis.graph.calls[1].name.as_deref(), Some("r#helper"));
    let shadowed = analyze("fn helper(){} fn run(r#helper:fn()){helper();}");
    all_reason(&shadowed, "shadowed_name");
}

#[test]
fn nfc_equivalent_definition_and_call_spellings_are_linked() {
    let analysis = analyze("fn café(){} fn run(){cafe\u{301}();}");
    assert_eq!(analysis.links.associations[0].status, "linked");
    assert_eq!(analysis.graph.calls[0].name.as_deref(), Some("cafe\u{301}"));
    let target = analysis
        .graph
        .contexts
        .iter()
        .find(|context| context.id == analysis.links.associations[0].target.as_deref().unwrap())
        .unwrap();
    assert_eq!(target.name.as_deref(), Some("café"));
}

#[test]
fn nfc_equivalent_bindings_are_not_missed_when_display_spellings_differ() {
    for source in [
        "fn café(){} fn run(){let cafe\u{301}=0; café();}",
        "fn café(){} fn run(r#cafe\u{301}:fn()){café();}",
        "fn K(){} fn run(){let K=0; K();}",
    ] {
        all_reason(&analyze(source), "shadowed_name");
    }
}

#[test]
fn normalized_duplicate_definitions_are_ambiguous_not_arbitrarily_selected() {
    let analysis = analyze("fn café(){} fn cafe\u{301}(){} fn run(){café();}");
    all_reason(&analysis, "ambiguous_name");
}

#[test]
fn case_sensitive_names_remain_distinct() {
    let analysis = analyze("fn helper(){} fn Helper(){} fn run(){helper();Helper();}");
    let links = &analysis.links.associations;
    assert_eq!(links[0].status, "linked");
    assert_eq!(links[1].status, "linked");
    assert_ne!(links[0].target, links[1].target);
}
