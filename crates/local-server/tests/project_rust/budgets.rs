//! 读取预算与纯路径预算分别归类，任何失败均清空全部证据。

use super::{
    observe_with,
    test_support::{Fixture, empty},
};

#[test]
fn per_file_byte_budget_accepts_exactly_64_kib() {
    let f = Fixture::new();
    let content = format!("//{}", "x".repeat(64 * 1024 - 2));
    f.write("src/api.rs", &content);
    assert_eq!(f.observe().status, "ready");
    f.write("src/api.rs", &format!("{content}x"));
    empty(&f.observe(), "unavailable");
}

#[test]
fn total_byte_budget_accepts_512_kib_and_rejects_one_more_byte() {
    let f = Fixture::new();
    let mut paths = Vec::new();
    for index in 0..8 {
        let path = format!("src/p{index}.rs");
        f.write(&path, &format!("//{}", "x".repeat(64 * 1024 - 2)));
        paths.push(path);
    }
    let observe = |paths| {
        f.repository()
            .with_rust_call_sites(paths)
            .unwrap()
            .observe_rust_call_sites()
    };
    assert_eq!(observe(paths.clone()).status, "ready");
    f.write("src/z.rs", "");
    paths.push("src/z.rs".into());
    assert_eq!(observe(paths.clone()).status, "ready");
    f.write("src/z.rs", " ");
    empty(&observe(paths), "unavailable");
}

#[test]
fn path_count_length_and_depth_budgets_fail_before_any_io() {
    for paths in [
        (0..17).map(|index| format!("src/p{index}.rs")).collect(),
        vec![format!("{}.rs", "x".repeat(1022))],
        vec![format!("{}a.rs", "d/".repeat(16))],
    ] {
        let result = observe_with(
            &paths,
            |_, _| panic!("不应读取超限计划"),
            || panic!("不应复核根"),
        );
        empty(&result, "unsupported");
    }
}
