//! 显式成员计划、完整输入集合与根内规范目录边界。

#[path = "support/cargo.rs"]
mod support;

use studiolo_workbench::{
    CargoDependencyErrorKind::{Invalid, Unsupported},
    cargo_manifest_paths, derive_cargo_dependencies,
};
use support::*;

#[test]
fn package_virtual_and_combined_roots_produce_unique_sorted_plans() {
    assert_eq!(
        cargo_manifest_paths(&package("root", "")).unwrap(),
        ["Cargo.toml"]
    );
    let workspace = "[workspace]\nmembers=['z','a/./inner/..']\n";
    assert_eq!(
        cargo_manifest_paths(workspace).unwrap(),
        ["Cargo.toml", "a/Cargo.toml", "z/Cargo.toml"]
    );
    let combined = format!(
        "{}[workspace]\nmembers=['.', 'child']\n",
        package("root", "")
    );
    assert_eq!(
        cargo_manifest_paths(&combined).unwrap(),
        ["Cargo.toml", "child/Cargo.toml"]
    );
}

#[test]
fn duplicates_case_collisions_and_unreadable_directory_spellings_reject_plans() {
    for members in [
        "['a','a']",
        "['a','a/./']",
        "['A','a']",
        "['Src/a','src/b']",
        "['.hidden']",
        "['a//b']",
        "['/absolute']",
        "['C:/disk']",
        "['a\\\\b']",
        "['a/%2e%2e']",
        "['a/\u{200b}hidden']",
        "[42]",
    ] {
        error(
            cargo_manifest_paths(&format!("[workspace]\nmembers={members}\n")),
            Invalid,
        );
    }
}

#[test]
fn glob_exclusion_override_and_root_escape_are_unsupported() {
    for root in [
        "[workspace]\nmembers=['crates/*']\n",
        "[workspace]\nmembers=['../external']\n",
        "[workspace]\nmembers=['app']\nexclude=['app']\n",
        "[package]\nname='app'\n[patch.crates-io]\nfoo={path='foo'}\n",
        "[package]\nname='app'\n[replace]\n'foo:1.0'={path='foo'}\n",
    ] {
        error(cargo_manifest_paths(root), Unsupported);
    }
}

#[test]
fn malformed_or_empty_roots_do_not_become_successful_empty_graphs() {
    for root in [
        "",
        "[workspace]\nmembers=[]\n",
        "[workspace]\nmembers=['.']\n",
        "[package]\n",
        "[workspace]\nmembers='app'\n",
        "[workspace]\nexclude=true\n",
        "PRIVATE_VALUE = [",
    ] {
        error(cargo_manifest_paths(root), Invalid);
    }
}

#[test]
fn input_set_must_match_recomputed_root_plan_and_order_does_not_change_evidence() {
    let inputs = pair("[dependencies]\ncore.workspace=true");
    let expected = derive_cargo_dependencies(&inputs).unwrap();
    let mut reordered = inputs.clone();
    reordered.reverse();
    assert_eq!(derive_cargo_dependencies(&reordered).unwrap(), expected);
    error(derive_cargo_dependencies(&inputs[..2]), Invalid);
    let mut extra = inputs.clone();
    extra.push(manifest("outside/Cargo.toml", &package("outside", "")));
    error(derive_cargo_dependencies(&extra), Invalid);
    let mut duplicate = inputs.clone();
    duplicate.push(inputs[0].clone());
    error(derive_cargo_dependencies(&duplicate), Invalid);
    let mut changed_root = inputs;
    changed_root[0].content = "[workspace]\nmembers=['app']".to_owned();
    error(derive_cargo_dependencies(&changed_root), Invalid);
}

#[test]
fn nested_workspace_external_owner_and_missing_or_duplicate_package_names_fail() {
    for (body, kind) in [
        ("[workspace]\n", Unsupported),
        ("workspace='../core'\n", Unsupported),
    ] {
        let mut inputs = pair("");
        inputs[1].content = if body.starts_with("workspace") {
            format!("[package]\nname='app'\n{body}")
        } else {
            package("app", body)
        };
        error(derive_cargo_dependencies(&inputs), kind);
    }
    let mut missing = pair("");
    missing[1].content = "[dependencies]\n".to_owned();
    error(derive_cargo_dependencies(&missing), Invalid);
    let mut duplicate = pair("");
    duplicate[1].content = package("core", "");
    error(derive_cargo_dependencies(&duplicate), Invalid);
    let mut valid_owner = pair("");
    valid_owner[1].content = "[package]\nname='app'\nworkspace='..'\n".to_owned();
    assert_eq!(
        derive_cargo_dependencies(&valid_owner)
            .unwrap()
            .packages
            .len(),
        2
    );
}
