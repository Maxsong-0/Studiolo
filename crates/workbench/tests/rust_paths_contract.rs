//! 路径、文件集合和首批字节预算必须在解析前完整验证。

#[path = "support/rust.rs"]
mod support;

use studiolo_workbench::{
    RustSourceErrorKind::{Invalid, Unsupported},
    derive_rust_call_sites, validate_rust_source_paths,
};
use support::*;

fn paths(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn visible_nfc_paths_are_sorted_without_rewriting() {
    let values = paths(&["源码/入口.rs", "src/with space.rs", "src/lib.rs", "é.rs"]);
    let mut sorted = values.clone();
    sorted.sort();
    assert_eq!(validate_rust_source_paths(&values).unwrap(), sorted);
}

#[test]
fn malformed_hidden_encoded_or_non_rust_paths_are_rejected() {
    for path in [
        "",
        "/src/a.rs",
        "C:/a.rs",
        "C:a.rs",
        "./a.rs",
        "src/../a.rs",
        "src//a.rs",
        "src\\a.rs",
        ".hidden/a.rs",
        "src/.a.rs",
        "src/a.RS",
        "src/a.rs/",
        "src/a%2fb.rs",
        "src/a%5cb.rs",
        "src/%2ea.rs",
        "src/a\0.rs",
        "src/a\n.rs",
        "e\u{301}.rs",
    ] {
        error(validate_rust_source_paths(&paths(&[path])), Invalid);
    }
}

#[test]
fn unicode_controls_formats_and_separators_are_not_visible_paths() {
    for value in [
        '\u{ad}',
        '\u{600}',
        '\u{61c}',
        '\u{890}',
        '\u{180e}',
        '\u{200b}',
        '\u{2028}',
        '\u{2029}',
        '\u{202e}',
        '\u{2066}',
        '\u{feff}',
        '\u{fff9}',
        '\u{110bd}',
        '\u{13430}',
        '\u{1bca0}',
        '\u{1d173}',
        '\u{e0001}',
        '\u{e0020}',
    ] {
        error(
            validate_rust_source_paths(&[format!("src/a{value}.rs")]),
            Invalid,
        );
    }
}

#[test]
fn duplicates_prefix_case_collisions_and_file_directory_conflicts_are_atomic() {
    for values in [
        paths(&["src/a.rs", "src/a.rs"]),
        paths(&["Src/a.rs", "src/b.rs"]),
        paths(&["src/Inner/a.rs", "src/inner/b.rs"]),
        paths(&["src/a.rs", "src/A.rs"]),
        paths(&["src/a.rs", "src/a.rs/b.rs"]),
    ] {
        error(validate_rust_source_paths(&values), Invalid);
    }
    error(
        derive_rust_call_sites(&[input("src/a.rs", "fn a(){}"), input("src/a.rs", "fn b(){}")]),
        Invalid,
    );
}

#[test]
fn file_count_path_bytes_and_components_have_inclusive_limits() {
    error(validate_rust_source_paths(&[]), Invalid);
    let paths = (0..16)
        .map(|index| format!("src/f{index}.rs"))
        .collect::<Vec<_>>();
    assert_eq!(validate_rust_source_paths(&paths).unwrap().len(), 16);
    let mut excessive = paths;
    excessive.push("extra.rs".to_owned());
    error(validate_rust_source_paths(&excessive), Unsupported);
    assert!(validate_rust_source_paths(&[format!("{}.rs", "a".repeat(1021))]).is_ok());
    error(
        validate_rust_source_paths(&[format!("{}.rs", "a".repeat(1022))]),
        Unsupported,
    );
    assert!(validate_rust_source_paths(&[format!("{}a.rs", "d/".repeat(15))]).is_ok());
    error(
        validate_rust_source_paths(&[format!("{}a.rs", "d/".repeat(16))]),
        Unsupported,
    );
}

#[test]
fn empty_files_and_exact_source_byte_budget_are_valid() {
    let empty = derive_rust_call_sites(&[input("empty.rs", "")]).unwrap();
    assert_eq!(empty.contexts.len(), 1);
    assert_eq!(empty.contexts[0].id, "f0:root");
    assert!(empty.calls.is_empty());
    let inputs = (0..8)
        .map(|index| input(&format!("f{index}.rs"), &" ".repeat(65536)))
        .collect::<Vec<_>>();
    let graph = derive_rust_call_sites(&inputs).unwrap();
    assert_eq!(
        graph.files.iter().map(|file| file.bytes).sum::<u32>(),
        512 * 1024
    );
}

#[test]
fn oversized_file_or_batch_never_returns_a_partial_graph() {
    error(
        derive_rust_call_sites(&[input("a.rs", &" ".repeat(65537))]),
        Unsupported,
    );
    let inputs = (0..9)
        .map(|index| input(&format!("f{index}.rs"), &" ".repeat(65536)))
        .collect::<Vec<_>>();
    error(derive_rust_call_sites(&inputs), Unsupported);
    error(derive_rust_call_sites(&[]), Invalid);
}
