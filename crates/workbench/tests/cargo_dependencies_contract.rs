//! 包依赖身份、声明种类、继承与精确原文证据。

#[path = "support/cargo.rs"]
mod support;

use studiolo_workbench::{
    CargoDependencyErrorKind::{Invalid, Unsupported},
    derive_cargo_dependencies,
};
use support::*;

#[test]
fn inherited_dotted_inline_target_and_table_dependencies_retain_both_sources() {
    let inputs = vec![
        manifest(
            "Cargo.toml",
            "# 根\n[workspace]\nmembers=['app','core']\n[workspace.dependencies]\nrenamed={package='core',path='core'}\n",
        ),
        manifest(
            "app/Cargo.toml",
            "# 使用方\n[package]\nname='app'\n[dependencies]\nrenamed.workspace=true\n[dev-dependencies.core]\npath='../core'\n[target.'cfg(unix)'.build-dependencies]\ncore={path='../core',optional=true,features=['demo']}\n",
        ),
        manifest("core/Cargo.toml", "[package]\nname='core'\n"),
    ];
    let graph = derive_cargo_dependencies(&inputs).unwrap();
    assert_eq!(graph.packages[0].source.line, 3);
    assert_eq!(graph.dependencies.len(), 3);
    let normal = graph
        .dependencies
        .iter()
        .find(|edge| edge.kind == "normal")
        .unwrap();
    assert_eq!(normal.from, "app");
    assert_eq!(normal.to, "core");
    assert_eq!(normal.alias, "renamed");
    assert_eq!(normal.source.path, "app/Cargo.toml");
    assert_eq!(normal.source.line, 5);
    assert_eq!(normal.inherited_from.as_ref().unwrap().path, "Cargo.toml");
    assert_eq!(normal.inherited_from.as_ref().unwrap().line, 5);
    let dev = graph
        .dependencies
        .iter()
        .find(|edge| edge.kind == "dev")
        .unwrap();
    assert_eq!(dev.source.line, 6);
    assert!(dev.inherited_from.is_none());
    let build = &graph.dependencies[0];
    assert_eq!(build.kind, "build");
    assert_eq!(build.source.line, 9);
    assert_eq!(build.target.as_deref(), Some("cfg(unix)"));
    assert!(build.optional);
}

#[test]
fn root_package_self_edges_cycles_and_same_target_through_two_aliases_are_retained() {
    let root = package("root", "[dependencies]\nroot={path='.'}");
    let graph = derive_cargo_dependencies(&[manifest("Cargo.toml", &root)]).unwrap();
    assert_eq!(graph.packages[0].id, ".");
    assert_eq!(graph.dependencies[0].to, ".");
    let mut inputs =
        pair("[dependencies]\ncore={path='../core'}\nrenamed={path='../core',package='core'}");
    inputs[2].content = package("core", "[dependencies]\napp={path='../app'}");
    assert_eq!(
        derive_cargo_dependencies(&inputs)
            .unwrap()
            .dependencies
            .len(),
        3
    );
}

#[test]
fn non_path_dependencies_count_declarations_and_do_not_match_local_names() {
    let inputs = pair(
        "[dependencies]\ncore='1'\nexternal.workspace=true\nremote={git='https://PRIVATE_VALUE.invalid/repo#fragment',rev='revision'}\n[dev-dependencies]\ncore={version='1',registry='alternate'}\n[target.'cfg(unix)'.build-dependencies]\nexternal.workspace=true",
    );
    let graph = derive_cargo_dependencies(&inputs).unwrap();
    assert_eq!(graph.external_dependencies, 5);
    assert!(graph.dependencies.is_empty());
    assert!(!format!("{graph:?}").contains("PRIVATE_VALUE"));
}

#[test]
fn inherited_optional_features_and_root_default_features_remain_declarations() {
    let mut inputs = pair(
        "[dependencies]\ncore={workspace=true,optional=true,features=['own']}\n[dev-dependencies]\ncore.workspace=true",
    );
    inputs[0].content = "[workspace]\nmembers=['app','core']\n[workspace.dependencies]\ncore={path='core',default-features=false,features=['base']}\nunused='1'".to_owned();
    let graph = derive_cargo_dependencies(&inputs).unwrap();
    assert_eq!(graph.external_dependencies, 0);
    assert!(!graph.dependencies[0].optional);
    assert!(graph.dependencies[1].optional);
    assert!(
        graph
            .dependencies
            .iter()
            .all(|edge| edge.inherited_from.is_some())
    );
}

#[test]
fn path_outside_members_rename_mismatch_and_missing_inheritance_are_not_partial_successes() {
    for (body, kind) in [
        ("[dependencies]\ncore={path='../unlisted'}", Unsupported),
        ("[dependencies]\ncore={path='../../outside'}", Unsupported),
        ("[dependencies]\nalias={path='../core'}", Invalid),
        (
            "[dependencies]\ncore={path='../core',package='other'}",
            Invalid,
        ),
        ("[dependencies]\nmissing.workspace=true", Invalid),
    ] {
        error(derive_cargo_dependencies(&pair(body)), kind);
    }
}

#[test]
fn malformed_types_unknown_dependency_semantics_and_inheritance_overrides_are_rejected() {
    for dependency in [
        "{}",
        "42",
        "{path=true}",
        "{optional=true}",
        "{path='../core',optional='yes'}",
        "{path='../core',features=[42]}",
        "{path='../core',default-features=1}",
        "{path='../core',git='repo'}",
        "{git='repo',branch='a',tag='b'}",
        "{version='1',rev='x'}",
        "{workspace=false}",
    ] {
        error(
            derive_cargo_dependencies(&pair(&format!("[dependencies]\ncore={dependency}"))),
            Invalid,
        );
    }
    for dependency in [
        "{path='../core',base='workspace'}",
        "{version='1',artifact='bin'}",
        "{workspace=true,default-features=false}",
        "{workspace=true,path='../core'}",
        "{workspace=true,package='core'}",
    ] {
        error(
            derive_cargo_dependencies(&pair(&format!("[dependencies]\ncore={dependency}"))),
            Unsupported,
        );
    }
    let mut root_optional = pair("[dependencies]\ncore.workspace=true");
    root_optional[0].content = root_optional[0]
        .content
        .replace("path='core'", "path='core',optional=false");
    error(derive_cargo_dependencies(&root_optional), Invalid);
}

#[test]
fn unicode_crlf_comments_and_quoted_keys_preserve_real_toml_locations() {
    let mut inputs = pair("");
    inputs[1].content = "# 中文 # = { }\r\n[package]\r\nname='app'\r\n[dependencies]\r\n\"core\" = {\r\n path='../core', # 多行 TOML 1.1 inline table\r\n features=['a,b#c'],\r\n}\r\n".to_owned();
    let graph = derive_cargo_dependencies(&inputs).unwrap();
    assert_eq!(graph.dependencies[0].source.line, 5);
    assert_eq!(graph.packages[0].source.line, 3);
}

#[test]
fn malformed_toml_targets_and_unsupported_legacy_sections_are_explicit_errors() {
    for body in [
        "[dependencies]\ncore=PRIVATE_VALUE",
        "[dependencies]\ncore='1'\ncore='2'",
        "dependencies=[]",
        "[target.'']\ndependencies={core={path='../core'}}",
        "[target.'\u{2028}']\ndependencies={core={path='../core'}}",
        "[target.'\u{2029}']\ndependencies={core={path='../core'}}",
    ] {
        let inputs = if body == "dependencies=[]" {
            vec![manifest(
                "Cargo.toml",
                &format!("dependencies=[]\n{}", package("app", "")),
            )]
        } else {
            pair(body)
        };
        error(derive_cargo_dependencies(&inputs), Invalid);
    }
    for section in [
        "dev_dependencies",
        "build_dependencies",
        "target.'cfg(unix)'.dev_dependencies",
        "target.'cfg(unix)'.build_dependencies",
    ] {
        error(
            derive_cargo_dependencies(&pair(&format!("[{section}]\ncore={{path='../core'}}"))),
            Unsupported,
        );
    }
}

#[test]
fn complete_fictional_workspace_keeps_three_packages_five_internal_and_two_external_declarations() {
    let inputs = vec![
        manifest(
            "Cargo.toml",
            include_str!("../../../examples/m3-cargo-dependencies/repository/Cargo.toml"),
        ),
        manifest(
            "src/catalog/Cargo.toml",
            include_str!(
                "../../../examples/m3-cargo-dependencies/repository/src/catalog/Cargo.toml"
            ),
        ),
        manifest(
            "src/search/Cargo.toml",
            include_str!(
                "../../../examples/m3-cargo-dependencies/repository/src/search/Cargo.toml"
            ),
        ),
        manifest(
            "tools/Cargo.toml",
            include_str!("../../../examples/m3-cargo-dependencies/repository/tools/Cargo.toml"),
        ),
    ];
    let graph = derive_cargo_dependencies(&inputs).unwrap();
    assert_eq!(graph.packages.len(), 3);
    assert_eq!(graph.dependencies.len(), 5);
    assert_eq!(graph.external_dependencies, 2);
    assert!(graph.dependencies.iter().any(|edge| edge.optional));
    assert!(graph.dependencies.iter().any(|edge| edge.target.is_some()));
}
