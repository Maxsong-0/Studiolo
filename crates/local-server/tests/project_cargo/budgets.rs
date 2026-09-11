//! 内容预算是每轮完整批次，不能耗尽预算后返回部分包图。

use super::test_support as support;

use std::fs;

use support::{Fixture, ROOT, empty};

#[test]
fn a_manifest_accepts_64_kib_and_rejects_the_next_byte() {
    let f = Fixture::new();
    let content = format!("{ROOT}#{}", "x".repeat(64 * 1024 - ROOT.len() - 1));
    f.write("Cargo.toml", &content);
    assert_eq!(f.observe().status, "ready");
    f.write("Cargo.toml", &format!("{content}x"));
    empty(&f.observe(), "unavailable");
}

#[test]
fn aggregate_content_limit_is_not_replaced_by_the_package_count_limit() {
    let f = Fixture::new();
    let members = (0..32)
        .map(|index| format!("\"p{index}\""))
        .collect::<Vec<_>>()
        .join(", ");
    f.write(
        "Cargo.toml",
        &format!("[workspace]\nmembers = [{members}]\n"),
    );
    for index in 0..32 {
        fs::create_dir(f.root.path().join(format!("p{index}"))).unwrap();
        let base = format!("[package]\nname = \"p{index}\"\nversion = \"0.1.0\"\n#");
        let body = format!("{base}{}", "x".repeat(64 * 1024 - base.len()));
        f.write(&format!("p{index}/Cargo.toml"), &body);
    }
    empty(&f.observe(), "unavailable");
}

#[test]
fn oversized_or_deep_member_plans_fail_before_attempting_the_member_read() {
    use super::observe_with;
    for member in ["d".repeat(1025), vec!["d"; 17].join("/")] {
        let f = Fixture::new();
        f.write(
            "Cargo.toml",
            &format!("[workspace]\nmembers = [\"{member}\"]\n"),
        );
        let reader = f.reader();
        let mut reads = 0;
        let result = observe_with(
            |path, limit| {
                reads += 1;
                reader.read_file(path, limit)
            },
            || Ok(()),
        );
        assert!(matches!(result.status, "invalid" | "unsupported"));
        assert!(result.graph.is_none());
        assert_eq!(reads, 1);
    }
}
