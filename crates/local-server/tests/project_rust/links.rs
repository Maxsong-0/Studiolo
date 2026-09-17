//! 调用与定义来自同一正文批次；内容身份不冒充 Vault 代次或文件系统事务。

use std::fs;

use super::test_support::{Fixture, empty, paths, ready};

const API: &str = "fn helper() {}\nfn serve() { helper(); helper(); external(); }\n";
const MODEL: &str = "fn external() {}\nfn serve() { external(); }\n";

fn prepared() -> Fixture {
    let f = Fixture::new();
    f.write("src/api.rs", API);
    f.write("src/model.rs", MODEL);
    f
}

#[test]
fn every_call_has_one_same_batch_association_and_linked_targets_stay_in_their_file() {
    let f = prepared();
    let result = f.observe();
    ready(&result);
    let graph = result.graph.as_ref().unwrap();
    let links = result.links.as_ref().unwrap();
    assert_eq!(graph.calls.len(), 4);
    assert_eq!(
        links
            .associations
            .iter()
            .filter(|row| row.status == "linked")
            .count(),
        3
    );
    assert_eq!(links.associations[0].target, links.associations[1].target);
    assert_eq!(links.associations[2].reason.as_deref(), Some("not_found"));
    assert_ne!(links.associations[0].target, links.associations[3].target);
}

#[test]
fn same_length_edits_keep_location_ids_but_replace_the_batch_and_its_target_names() {
    let f = prepared();
    let repository = f.repository().with_rust_call_sites(paths()).unwrap();
    let before = repository.observe_rust_call_sites();
    ready(&before);
    let changed = API.replace("helper", "worker");
    assert_eq!(changed.len(), API.len());
    f.write("src/api.rs", &changed);
    let after = repository.observe_rust_call_sites();
    ready(&after);
    let ids = |value: &super::RustCallObservation| {
        value
            .graph
            .as_ref()
            .unwrap()
            .calls
            .iter()
            .map(|call| call.id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&before), ids(&after));
    assert_ne!(
        before.links.as_ref().unwrap().batch_id,
        after.links.as_ref().unwrap().batch_id
    );
    assert_eq!(
        after.graph.as_ref().unwrap().calls[0].name.as_deref(),
        Some("worker")
    );
    let target_id = after.links.as_ref().unwrap().associations[0]
        .target
        .as_ref()
        .unwrap();
    let target = after
        .graph
        .as_ref()
        .unwrap()
        .contexts
        .iter()
        .find(|row| &row.id == target_id)
        .unwrap();
    assert_eq!(target.name.as_deref(), Some("worker"));
    f.write("src/api.rs", API);
    let restored = repository.observe_rust_call_sites();
    ready(&restored);
    assert_eq!(
        before.links.as_ref().unwrap().batch_id,
        restored.links.as_ref().unwrap().batch_id
    );
}

#[test]
fn hidden_argument_changes_affect_the_batch_without_leaking_the_body() {
    let f = prepared();
    let body = "fn helper(_: &str) {}\nfn serve() { helper(\"ARGUMENT_A\"); }\n";
    f.write("src/api.rs", body);
    let before = f.observe();
    f.write("src/api.rs", &body.replace("ARGUMENT_A", "ARGUMENT_B"));
    let after = f.observe();
    ready(&before);
    ready(&after);
    assert_eq!(before.graph, after.graph);
    assert_ne!(
        before.links.as_ref().unwrap().batch_id,
        after.links.as_ref().unwrap().batch_id
    );
    for value in [&before, &after] {
        let encoded = serde_json::to_string(value).unwrap();
        assert!(!encoded.contains("ARGUMENT_"));
        assert!(!encoded.contains(f.root.path().to_str().unwrap()));
    }
}

#[test]
fn failures_never_keep_prior_links_and_recovery_rebuilds_the_current_batch() {
    let f = prepared();
    let repository = f.repository().with_rust_call_sites(paths()).unwrap();
    let before = repository.observe_rust_call_sites();
    ready(&before);
    for (body, status) in [
        ("fn SECRET_SYNTAX( {".to_owned(), "invalid"),
        (
            format!("fn deep() {{ {}0{}; }}", "(".repeat(80), ")".repeat(80)),
            "unsupported",
        ),
    ] {
        f.write("src/api.rs", &body);
        empty(&repository.observe_rust_call_sites(), status);
    }
    fs::remove_file(f.root.path().join("src/api.rs")).unwrap();
    empty(&repository.observe_rust_call_sites(), "unavailable");
    f.write("src/api.rs", &API.replace("helper", "worker"));
    let recovered = repository.observe_rust_call_sites();
    ready(&recovered);
    assert_ne!(
        before.links.as_ref().unwrap().batch_id,
        recovered.links.as_ref().unwrap().batch_id
    );
}
