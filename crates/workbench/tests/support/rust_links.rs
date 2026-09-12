//! T-045 只核对本批引用身份，不把同名视为关系。

#![allow(dead_code)]

use std::collections::BTreeMap;

use studiolo_workbench::{
    RustCallAnalysis, RustCallAssociation, RustSourceInput, derive_rust_call_analysis,
};

pub fn input(path: &str, content: &str) -> RustSourceInput {
    RustSourceInput {
        path: path.to_owned(),
        content: content.to_owned(),
    }
}

pub fn analyze(source: &str) -> RustCallAnalysis {
    let analysis = derive_rust_call_analysis(&[input("a.rs", source)]).unwrap();
    references(&analysis);
    analysis
}

pub fn named<'a>(analysis: &'a RustCallAnalysis, name: &str) -> Vec<&'a RustCallAssociation> {
    analysis
        .graph
        .calls
        .iter()
        .zip(&analysis.links.associations)
        .filter(|(call, _)| call.name.as_deref() == Some(name))
        .map(|(_, link)| link)
        .collect()
}

pub fn reason(link: &RustCallAssociation, reason: &str) {
    assert_eq!(link.status, "unresolved");
    assert_eq!(link.reason.as_deref(), Some(reason));
    assert!(link.target.is_none());
}

pub fn all_reason(analysis: &RustCallAnalysis, expected: &str) {
    assert!(!analysis.links.associations.is_empty());
    for link in &analysis.links.associations {
        reason(link, expected);
    }
}

pub fn references(analysis: &RustCallAnalysis) {
    let graph = &analysis.graph;
    assert_eq!(analysis.links.version, 1);
    assert_eq!(analysis.links.batch_id.len(), 64);
    assert!(
        analysis
            .links
            .batch_id
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    );
    assert_eq!(graph.calls.len(), analysis.links.associations.len());
    let contexts = graph
        .contexts
        .iter()
        .map(|context| (context.id.as_str(), context))
        .collect::<BTreeMap<_, _>>();
    for (call, link) in graph.calls.iter().zip(&analysis.links.associations) {
        assert_eq!(link.call_id, call.id);
        match link.status.as_str() {
            "linked" => {
                assert!(link.reason.is_none());
                let target = contexts[link.target.as_deref().unwrap()];
                assert_eq!(target.kind, "function");
                assert_eq!(target.source.path, call.source.path);
            }
            "unresolved" => {
                assert!(link.target.is_none());
                assert!(matches!(
                    link.reason.as_deref(),
                    Some(
                        "callee_form"
                            | "context"
                            | "opaque_scope"
                            | "shadowed_name"
                            | "ambiguous_name"
                            | "generic_target"
                            | "not_found"
                    )
                ));
            }
            other => panic!("意外关联状态：{other}"),
        }
    }
}
