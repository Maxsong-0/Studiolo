//! 同文件正向关联的虚构源码；API 响应只含标识、位置和内容批标识。

use serde_json::Value;

use crate::{cargo, fixture::Fixture, rust};

pub const API: &str = "fn helper(_: &str) {}\nfn serve() { helper(\"ARGUMENT_SECRET\"); helper(\"ARGUMENT_SECRET\"); external(); }\n";

pub fn prepared() -> Fixture {
    let f = rust::prepared();
    cargo::write(&f, "src/api.rs", API);
    cargo::write(
        &f,
        "src/model.rs",
        "fn external() {}\nfn serve() { external(); }\n",
    );
    f
}

pub fn empty(value: &Value, status: &str) {
    rust::empty(value, status);
    assert_eq!(value["rust_call_sites"].get("links"), Some(&Value::Null));
}

pub fn ready(value: &Value) -> &Value {
    let source = &value["rust_call_sites"];
    assert_eq!(source["status"], "ready");
    assert!(
        source["observed_at"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
            > 0
    );
    assert!(source["diagnostics"].as_array().unwrap().is_empty());
    let links = &source["links"];
    assert_eq!(links["version"], 1);
    let batch = links["batch_id"].as_str().unwrap();
    assert_eq!(batch.len(), 64);
    assert!(
        batch
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    let calls = source["graph"]["calls"].as_array().unwrap();
    let contexts = source["graph"]["contexts"].as_array().unwrap();
    let rows = links["associations"].as_array().unwrap();
    assert_eq!(calls.len(), rows.len());
    for (call, row) in calls.iter().zip(rows) {
        assert_eq!(row["call_id"], call["id"]);
        if row["status"] == "linked" {
            assert_eq!(row.get("reason"), Some(&Value::Null));
            let target = contexts
                .iter()
                .find(|context| context["id"] == row["target"])
                .unwrap();
            assert_eq!(target["kind"], "function");
            assert_eq!(target["source"]["path"], call["source"]["path"]);
        } else {
            assert_eq!(row["status"], "unresolved");
            assert_eq!(row.get("target"), Some(&Value::Null));
            assert!(row["reason"].is_string());
        }
    }
    links
}
