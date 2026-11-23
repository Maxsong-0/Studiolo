use super::fixture::*;
use serde_json::json;
use studiolo_host_config::{SyncFileKind as Kind, SyncScopeError as Error};

#[test]
fn config_rules_and_registration_lists_remain_bounded_by_count_and_sixteen_kib() {
    let f = Fixture::new();
    for (field, values) in [
        (
            "whole_folders",
            (0..65)
                .map(|i| folder(&format!("目录{i}")))
                .collect::<Vec<_>>(),
        ),
        (
            "whole_files",
            (0..129)
                .map(|i| file(&format!("文件{i}"), true, &[]))
                .collect(),
        ),
        (
            "generated_documents",
            (0..129).map(|i| json!(format!("生成{i}"))).collect(),
        ),
    ] {
        let mut value = f.value.clone();
        value["sync_scope"][field] = json!(values);
        f.write_value(&value);
        assert!(f.load().is_err());
    }
    let mut bytes = serde_json::to_vec(&f.value).unwrap();
    bytes.resize(16384, b' ');
    f.bytes(&bytes);
    assert!(f.load().unwrap().sync_scope().unwrap().is_some());
    bytes.push(b' ');
    f.bytes(&bytes);
    assert!(f.load().is_err());
}

#[test]
fn missing_entries_count_toward_whole_union_budget_and_failure_has_no_partial_result() {
    let f = Fixture::new();
    let scope = f.scope();
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let accepted: Vec<_> = (0..budget)
        .map(|i| missing(&format!("笔记/{i}.md")))
        .collect();
    assert_eq!(
        scope
            .approve_union(scope.binding(), &[], &accepted, &accepted)
            .unwrap()
            .paths()
            .len(),
        budget
    );
    let mut excessive = accepted.clone();
    excessive.push(missing("笔记/extra.md"));
    assert!(matches!(
        scope.approve_union(scope.binding(), &[], &excessive, &excessive),
        Err(Error::Budget)
    ));
    // 两侧各自都在预算内，合起来多一条照样拒绝。
    let left = &excessive[..budget / 2];
    let right = &excessive[budget / 2..];
    assert!(matches!(
        scope.approve_union(scope.binding(), &[], left, right),
        Err(Error::Budget)
    ));
    assert!(
        scope
            .approve_union(scope.binding(), &[], &accepted, &accepted)
            .is_ok()
    );
}

/// 名字索引按前缀计数：预算条路径各占一个目录时前缀是路径数的两倍多，索引上限随路径预算放大（BUG-047）。
#[test]
fn budget_paths_each_in_their_own_directory_fit_the_name_index() {
    let f = Fixture::new();
    let scope = f.scope();
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let spread: Vec<_> = (0..budget)
        .map(|i| missing(&format!("笔记/d{i}/n.md")))
        .collect();
    assert_eq!(
        scope
            .approve_union(scope.binding(), &spread, &spread, &spread)
            .unwrap()
            .paths()
            .len(),
        budget
    );
}

#[test]
fn individual_path_length_and_total_union_path_bytes_are_bounded() {
    let f = Fixture::new();
    let scope = f.scope();
    let too_long = vec![missing(&format!("笔记/{}", "a".repeat(1024)))];
    assert!(matches!(
        scope.approve_union(scope.binding(), &[], &too_long, &too_long),
        Err(Error::Budget)
    ));
    // 路径文字总长按每条 256 字节随路径预算放大（BUG-047）：每条恰好 512 字节，
    // 预算 × 256 字节恰好装满可过，再多一条即拒绝；条数仍在预算内，拒绝只因字节。
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let fits = budget * 256 / 512;
    let long_paths: Vec<_> = (0..=fits)
        .map(|i| missing(&format!("笔记/{i:05}{}", "a".repeat(500))))
        .collect();
    assert!(
        long_paths
            .iter()
            .all(|item| item.path.as_str().len() == 512)
    );
    assert!(long_paths.len() <= budget);
    let exact = &long_paths[..fits];
    assert!(
        scope
            .approve_union(scope.binding(), &[], exact, exact)
            .is_ok()
    );
    assert!(matches!(
        scope.approve_union(scope.binding(), &[], &long_paths, &long_paths),
        Err(Error::Budget)
    ));
}

#[test]
fn per_file_budget_covers_baseline_as_well_as_both_current_sides() {
    let f = Fixture::new();
    let scope = f.scope();
    let exact = vec![present("笔记/a.md", Kind::Markdown, 1048576)];
    assert!(
        scope
            .approve_union(scope.binding(), &exact, &exact, &exact)
            .is_ok()
    );
    let over = vec![present("笔记/a.md", Kind::Markdown, 1048577)];
    for (base, left, right) in [
        (&over, &exact, &exact),
        (&exact, &over, &exact),
        (&exact, &exact, &over),
    ] {
        assert!(matches!(
            scope.approve_union(scope.binding(), base, left, right),
            Err(Error::Budget)
        ));
    }
}

#[test]
fn total_observed_bytes_limit_is_independent_of_per_file_allowance() {
    let mut f = Fixture::new();
    f.value["sync_scope"]["whole_folders"][0]["max_bytes"] = json!(268435456);
    f.write();
    let scope = f.scope();
    let items: Vec<_> = (0..8)
        .map(|i| present(&format!("笔记/{i}.md"), Kind::Markdown, 268435456))
        .collect();
    assert!(
        scope
            .approve_union(scope.binding(), &[], &items, &items)
            .is_ok()
    );
    assert!(matches!(
        scope.approve_union(scope.binding(), &items, &items, &items),
        Err(Error::Budget)
    ));
}

#[test]
fn deeply_nested_input_is_bounded_even_below_union_path_text_budget() {
    let f = Fixture::new();
    let scope = f.scope();
    // 每条路径 63 个前缀：条数从预算推，派生前缀刚好超过索引上限（4 × 预算）；条数和路径文字都远在预算之内。
    let budget = studiolo_core::SYNC_PATH_BUDGET;
    let count = 4 * budget / 62 + 1;
    let items: Vec<_> = (0..count)
        .map(|i| {
            let nested = (0..60)
                .map(|j| format!("p{j}"))
                .collect::<Vec<_>>()
                .join("/");
            missing(&format!("笔记/{i}/{nested}/note.md"))
        })
        .collect();
    let text: usize = items.iter().map(|item| item.path.as_str().len()).sum();
    assert!(
        count < budget && text < budget * 256 / 4,
        "{count} 条、{text} 字节"
    );
    assert!(matches!(
        scope.approve_union(scope.binding(), &[], &items, &items),
        Err(Error::Budget)
    ));
}
