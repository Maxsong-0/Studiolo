use super::*;
use crate::seal_fixture::job;
fn document() -> serde_json::Value {
    serde_json::json!({"id": "ab".repeat(32), "tree": "cd".repeat(32), "paths": ["/sealed"], "tags": [JobTag::for_job(&job("job", 2)).unwrap().as_str()]})
}
#[test]
fn only_one_full_id_for_original_job_and_source_is_accepted() {
    let key = job("job", 2);
    let root = Path::new("/sealed");
    assert!(locate(b"[]", &key, root).unwrap().is_none());
    assert_eq!(
        locate(&serde_json::to_vec(&vec![document()]).unwrap(), &key, root)
            .unwrap()
            .unwrap()
            .as_str(),
        "ab".repeat(32)
    );
    for (field, value) in [
        ("id", serde_json::json!("abcdef12")),
        ("tree", serde_json::json!("abcdef12")),
        ("paths", serde_json::json!(["/other"])),
        ("paths", serde_json::json!(["/sealed", "/other"])),
        ("tags", serde_json::json!(["other"])),
        (
            "tags",
            serde_json::json!([JobTag::for_job(&key).unwrap().as_str(), "extra"]),
        ),
        ("excludes", serde_json::json!(["*.sqlite3"])),
        ("original", serde_json::json!("ef".repeat(32))),
    ] {
        let mut changed = document();
        changed[field] = value;
        assert!(
            locate(&serde_json::to_vec(&vec![changed]).unwrap(), &key, root).is_err(),
            "{field}"
        );
    }
    assert!(
        locate(
            &serde_json::to_vec(&vec![document(), document()]).unwrap(),
            &key,
            root
        )
        .is_err()
    );
    let raw = format!(
        "[{{\"id\":\"{}\",{}]",
        "ff".repeat(32),
        &document().to_string()[1..]
    );
    assert!(locate(raw.as_bytes(), &key, root).is_err(), "重复字段拒绝");
}
