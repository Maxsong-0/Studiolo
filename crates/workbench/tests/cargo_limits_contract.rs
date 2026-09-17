//! 每项预算均有可通过的边界和原子拒绝的下一项，不以截断掩盖缺失证据。

#[path = "support/cargo.rs"]
mod support;

use studiolo_workbench::{
    CargoDependencyErrorKind::{Invalid, Unsupported},
    cargo_manifest_paths, derive_cargo_dependencies,
};
use support::*;

fn workspace(count: usize) -> String {
    let members = (0..count)
        .map(|index| format!("'p{index}'"))
        .collect::<Vec<_>>()
        .join(",");
    format!("[workspace]\nmembers=[{members}]\n")
}

#[test]
fn package_manifest_and_path_budgets_include_the_positive_boundaries() {
    assert_eq!(cargo_manifest_paths(&workspace(64)).unwrap().len(), 65);
    error(cargo_manifest_paths(&workspace(65)), Unsupported);
    for path in ["p".repeat(1024), vec!["p"; 16].join("/")] {
        assert_eq!(
            cargo_manifest_paths(&format!("[workspace]\nmembers=['{path}']"))
                .unwrap()
                .len(),
            2
        );
    }
    for path in ["p".repeat(1025), vec!["p"; 17].join("/")] {
        error(
            cargo_manifest_paths(&format!("[workspace]\nmembers=['{path}']")),
            Unsupported,
        );
    }
    let mut inputs = vec![manifest("Cargo.toml", &workspace(64))];
    inputs.extend((0..64).map(|index| {
        manifest(
            &format!("p{index}/Cargo.toml"),
            &package(&format!("p{index}"), ""),
        )
    }));
    assert_eq!(
        derive_cargo_dependencies(&inputs).unwrap().packages.len(),
        64
    );
    inputs.push(manifest("extra/Cargo.toml", &package("extra", "")));
    error(derive_cargo_dependencies(&inputs), Unsupported);
}

#[test]
fn actual_dependency_entries_have_a_separate_2048_budget_from_unused_definitions() {
    let entries = (0..2048)
        .map(|index| format!("e{index}='1'\n"))
        .collect::<String>();
    let inputs = pair(&format!("[dependencies]\n{entries}"));
    assert_eq!(
        derive_cargo_dependencies(&inputs)
            .unwrap()
            .external_dependencies,
        2048
    );
    error(
        derive_cargo_dependencies(&pair(&format!("[dependencies]\n{entries}extra='1'\n"))),
        Unsupported,
    );
    let mut templates = pair("[dependencies]\ne0.workspace=true");
    templates[0].content =
        format!("[workspace]\nmembers=['app','core']\n[workspace.dependencies]\n{entries}");
    assert_eq!(
        derive_cargo_dependencies(&templates)
            .unwrap()
            .external_dependencies,
        1
    );
    templates[0].content.push_str("extra='1'\n");
    error(derive_cargo_dependencies(&templates), Unsupported);
}

#[test]
fn internal_edge_budget_rejects_the_257th_without_collapsing_distinct_aliases() {
    let edges = (0..256)
        .map(|index| format!("edge{index}={{path='../core',package='core'}}\n"))
        .collect::<String>();
    assert_eq!(
        derive_cargo_dependencies(&pair(&format!("[dependencies]\n{edges}")))
            .unwrap()
            .dependencies
            .len(),
        256
    );
    error(
        derive_cargo_dependencies(&pair(&format!(
            "[dependencies]\n{edges}extra={{path='../core',package='core'}}\n"
        ))),
        Unsupported,
    );
}

#[test]
fn entire_document_and_capture_bytes_include_comments_and_unused_content() {
    let base = package("root", "#");
    let exactly = format!("{base}{}", " ".repeat(64 * 1024 - base.len()));
    assert!(cargo_manifest_paths(&exactly).is_ok());
    error(cargo_manifest_paths(&format!("{exactly} ")), Unsupported);
    let root = workspace(64);
    let remaining = 2 * 1024 * 1024 - root.len();
    let mut inputs = vec![manifest("Cargo.toml", &root)];
    for index in 0..64 {
        let target = remaining / 64 + usize::from(index < remaining % 64);
        let base = package(&format!("p{index}"), "#");
        inputs.push(manifest(
            &format!("p{index}/Cargo.toml"),
            &format!("{base}{}", " ".repeat(target - base.len())),
        ));
    }
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.content.len())
            .sum::<usize>(),
        2 * 1024 * 1024
    );
    assert_eq!(
        derive_cargo_dependencies(&inputs).unwrap().packages.len(),
        64
    );
    inputs.last_mut().unwrap().content.push(' ');
    error(derive_cargo_dependencies(&inputs), Unsupported);
}

#[test]
fn identifiers_and_target_utf8_lengths_are_bounded_and_empty_or_multiline_values_fail() {
    let name = "a".repeat(64);
    assert!(cargo_manifest_paths(&package(&name, "")).is_ok());
    error(
        cargo_manifest_paths(&package(&format!("{name}a"), "")),
        Unsupported,
    );
    let target = format!("{}a", "中".repeat(85));
    let body = format!("[target.'{target}'.dependencies]\ncore={{path='../core'}}");
    assert_eq!(
        derive_cargo_dependencies(&pair(&body))
            .unwrap()
            .dependencies[0]
            .target
            .as_ref()
            .unwrap()
            .len(),
        256
    );
    error(
        derive_cargo_dependencies(&pair(&body.replace(&target, &format!("{target}a")))),
        Unsupported,
    );
    error(
        derive_cargo_dependencies(&pair(&format!("[dependencies]\n'{name}a'='1'"))),
        Unsupported,
    );
    for alias in ["", "含中文", "with space"] {
        error(
            derive_cargo_dependencies(&pair(&format!("[dependencies]\n'{alias}'='1'"))),
            Invalid,
        );
    }
}

#[test]
fn parser_recursion_limits_remain_enabled_and_errors_do_not_expose_input_values() {
    let nested = format!(
        "[package]\nname='PRIVATE_VALUE'\n[package.metadata]\nnested={}0{}\n",
        "[".repeat(100),
        "]".repeat(100)
    );
    error(cargo_manifest_paths(&nested), Invalid);
    error(
        cargo_manifest_paths("[package]\nname='PRIVATE_VALUE'\nname='other'\n"),
        Invalid,
    );
}
