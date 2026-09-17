//! Rust 纯派生契约共用的输入和图引用检查。

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use studiolo_workbench::{RustCallGraph, RustSourceError, RustSourceErrorKind, RustSourceInput};

pub fn input(path: &str, content: &str) -> RustSourceInput {
    RustSourceInput {
        path: path.to_owned(),
        content: content.to_owned(),
    }
}

pub fn error<T: std::fmt::Debug>(result: Result<T, RustSourceError>, kind: RustSourceErrorKind) {
    let error = result.unwrap_err();
    assert_eq!(error.kind, kind, "{error:?}");
    assert!(!error.message.contains("PRIVATE_VALUE"));
    assert!(!error.message.is_ascii());
}

pub fn references(graph: &RustCallGraph) {
    let contexts = graph
        .contexts
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(contexts.len(), graph.contexts.len());
    let calls = graph
        .calls
        .iter()
        .map(|item| &item.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(calls.len(), graph.calls.len());
    for context in &graph.contexts {
        let mut seen = BTreeSet::new();
        let mut next = Some(context);
        while let Some(current) = next {
            assert!(seen.insert(&current.id));
            assert_eq!(current.source.path, context.source.path);
            next = current.parent.as_deref().map(|id| contexts[id]);
        }
        assert_eq!(context.parent.is_none(), context.kind == "file");
    }
    for call in &graph.calls {
        assert_eq!(
            contexts[call.context.as_str()].source.path,
            call.source.path
        );
        assert_eq!(call.name.is_none(), call.kind == "expression");
    }
}
