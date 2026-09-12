//! 批身份与旧接口兼容性；黄金值由独立 Python hashlib 按公开编码计算。

#[path = "support/rust_links.rs"]
mod support;

use studiolo_workbench::{
    RustSourceErrorKind, RustSourceInput, derive_rust_call_analysis, derive_rust_call_sites,
};
use support::{input, references};

const SOURCE: &str = "fn helper(){} fn run(){helper();}";

#[test]
fn batch_hash_has_a_versioned_domain_lengths_and_canonical_file_order() {
    let inputs = [input("a.rs", SOURCE), input("z.rs", "")];
    let analysis = derive_rust_call_analysis(&inputs).unwrap();
    references(&analysis);
    assert_eq!(
        analysis.links.batch_id,
        "0e8b16cc178b80f4615a95d1a5e6ff1df1fc4039b54013fd6a5638490130873e"
    );
    let reordered = derive_rust_call_analysis(&[inputs[1].clone(), inputs[0].clone()]).unwrap();
    assert_eq!(analysis, reordered);
}

#[test]
fn same_length_rename_changes_identity_even_when_call_and_definition_ids_are_reused() {
    let before = derive_rust_call_analysis(&[input("a.rs", SOURCE)]).unwrap();
    let changed = SOURCE.replace("helper", "worker");
    assert_eq!(SOURCE.len(), changed.len());
    let after = derive_rust_call_analysis(&[input("a.rs", &changed)]).unwrap();
    references(&after);
    assert_eq!(before.graph.calls[0].id, after.graph.calls[0].id);
    assert_eq!(
        before.links.associations[0].target,
        after.links.associations[0].target
    );
    assert_ne!(before.links.batch_id, after.links.batch_id);
    assert_eq!(after.graph.calls[0].name.as_deref(), Some("worker"));
    let restored = derive_rust_call_analysis(&[input("a.rs", SOURCE)]).unwrap();
    assert_eq!(before, restored);
}

#[test]
fn path_and_hidden_argument_bytes_are_part_of_the_batch_identity() {
    let body = "fn run(){helper(\"PRIVATE_A\");} fn helper(_: &str){}";
    let original = derive_rust_call_analysis(&[input("a.rs", body)]).unwrap();
    let changed =
        derive_rust_call_analysis(&[input("a.rs", &body.replace("PRIVATE_A", "PRIVATE_B"))])
            .unwrap();
    assert_eq!(original.graph, changed.graph);
    assert_ne!(original.links.batch_id, changed.links.batch_id);
    let renamed = derive_rust_call_analysis(&[input("b.rs", body)]).unwrap();
    assert_ne!(original.links.batch_id, renamed.links.batch_id);
    let added = derive_rust_call_analysis(&[input("a.rs", ""), input("b.rs", body)]).unwrap();
    references(&added);
    assert!(added.graph.calls[0].id.starts_with("f1:"));
    assert_ne!(renamed.links.batch_id, added.links.batch_id);
}

#[test]
fn old_graph_output_is_unchanged_for_successful_plain_and_opaque_inputs() {
    for source in [
        "",
        SOURCE,
        "#[cfg(feature=\"hidden\")] fn f(){helper();}",
        "macro_rules! skip {()=>{phantom();}} fn f(){skip!();}",
        "fn helper<T>(){} fn f(){helper::<u8>();}",
        "fn f(){let callback=||helper(); let future=async {helper();};}",
    ] {
        let inputs = [input("a.rs", source)];
        let graph = derive_rust_call_sites(&inputs).unwrap();
        let analysis = derive_rust_call_analysis(&inputs).unwrap();
        references(&analysis);
        assert_eq!(graph, analysis.graph);
    }
}

fn same_error(inputs: &[RustSourceInput], kind: RustSourceErrorKind) {
    let old = derive_rust_call_sites(inputs).unwrap_err();
    let new = derive_rust_call_analysis(inputs).unwrap_err();
    assert_eq!(old.kind, kind);
    assert_eq!(old, new);
}

#[test]
fn invalid_batches_never_turn_into_partial_graphs_or_links() {
    same_error(&[], RustSourceErrorKind::Invalid);
    same_error(&[input("../a.rs", SOURCE)], RustSourceErrorKind::Invalid);
    same_error(
        &[input("a.rs", SOURCE), input("a.rs", "")],
        RustSourceErrorKind::Invalid,
    );
    same_error(
        &[input("a.rs", SOURCE), input("z.rs", "fn broken( {")],
        RustSourceErrorKind::Invalid,
    );
}

#[test]
fn file_byte_depth_context_and_call_budgets_keep_the_original_failure_contract() {
    let many_files = (0..17)
        .map(|n| input(&format!("f{n}.rs"), ""))
        .collect::<Vec<_>>();
    same_error(&many_files, RustSourceErrorKind::Unsupported);
    same_error(
        &[input("a.rs", &format!("//{}", "x".repeat(64 * 1024)))],
        RustSourceErrorKind::Unsupported,
    );
    let batch = (0..9)
        .map(|n| {
            input(
                &format!("f{n}.rs"),
                &format!("//{}", "x".repeat(64 * 1024 - 2)),
            )
        })
        .collect::<Vec<_>>();
    same_error(&batch, RustSourceErrorKind::Unsupported);
    let deep = format!("fn f(){{{}0{};}}", "(".repeat(80), ")".repeat(80));
    same_error(&[input("a.rs", &deep)], RustSourceErrorKind::Unsupported);
    let functions = (0..256)
        .map(|n| format!("fn f{n}(){{}}"))
        .collect::<String>();
    same_error(
        &[input("a.rs", &functions)],
        RustSourceErrorKind::Unsupported,
    );
    let calls = format!("fn f(){{{}}}", "helper();".repeat(1025));
    same_error(&[input("a.rs", &calls)], RustSourceErrorKind::Unsupported);
}
