//! 有来源的三类声明、引用完整性与项目隔离；不接文件系统或运行状态。

#[path = "support/architecture.rs"]
mod support;

use studiolo_workbench::WorkbenchReadModel;
use support::*;

#[test]
fn complete_fictional_sample_preserves_all_kinds_and_exact_sources() {
    let sample = include_str!(
        "../../../examples/m3-project-operations/vault/项目/示例工坊/01-框架/项目运作.md"
    );
    let value = architecture(sample);
    assert_eq!(value.status, "ready");
    assert!(value.diagnostics.is_empty());
    assert_eq!(value.source.as_ref().unwrap().path, PATH);
    assert_eq!(value.source.as_ref().unwrap().line, 1);
    assert_eq!(value.nodes.len(), 5);
    assert_eq!(value.relations.len(), 6);
    assert_eq!(value.flows.len(), 2);
    assert_eq!(
        value
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<Vec<_>>(),
        ["archive", "catalog", "export", "query", "reader"]
    );
    let query = &value.nodes[3];
    assert_eq!(query.label, "检索接口 <示例>");
    assert_eq!(query.kind, "interface");
    assert_eq!(query.directory.as_deref(), Some("src/search"));
    assert_eq!(query.source.line, 10);
    assert_eq!(value.nodes[0].kind, "data");
    assert_eq!(value.nodes[1].kind, "module");
    assert_eq!(value.nodes[4].kind, "external");
    assert!(value.nodes[4].directory.is_none());
    assert_eq!(value.flows[0].id, "history");
    assert_eq!(value.flows[1].steps[3].node, "query");
    assert_eq!(value.flows[1].steps[3].source.line, 33);
    for relation in &value.relations {
        assert_eq!(relation.source.path, PATH);
        assert!(
            sample
                .lines()
                .nth(relation.source.line as usize - 1)
                .unwrap()
                .starts_with(&format!("| {} | {} |", relation.from, relation.to))
        );
    }
}

#[test]
fn unknown_project_missing_document_and_other_project_content_are_distinct() {
    let content = declaration(NODE, "", "");
    let model = WorkbenchReadModel::build(&[
        document("项目/空项目/00-总览/INDEX.md", "# 已知项目"),
        document(PATH, &content),
    ])
    .unwrap();
    assert!(model.project_architecture("项目/不存在").is_none());
    assert!(
        model
            .project_architecture("项目/示例工坊/../空项目")
            .is_none()
    );
    let missing = model.project_architecture("项目/空项目").unwrap();
    assert_eq!(missing.status, "unavailable");
    assert!(missing.source.is_none());
    assert!(missing.nodes.is_empty());
    assert_eq!(
        model.project_architecture(PROJECT).unwrap().nodes[0]
            .source
            .path,
        PATH
    );
}

#[test]
fn immutable_generations_do_not_reuse_old_or_caller_modified_declarations() {
    let old = model(&declaration(NODE, "", ""));
    let new = model(&declaration(&NODE.replace("query", "catalog"), "", ""));
    let mut owned = old.project_architecture(PROJECT).unwrap();
    owned.nodes[0].id = "changed".to_owned();
    assert_eq!(
        old.project_architecture(PROJECT).unwrap().nodes[0].id,
        "query"
    );
    assert_eq!(
        new.project_architecture(PROJECT).unwrap().nodes[0].id,
        "catalog"
    );
    assert_eq!(
        old.project_operations(PROJECT).unwrap().declaration_status,
        "unavailable"
    );
}

#[test]
fn explicit_empty_relations_and_paths_are_valid_but_empty_nodes_are_not() {
    let value = architecture(&declaration(NODE, "", ""));
    assert_eq!(value.status, "ready");
    assert!(value.relations.is_empty());
    assert!(value.flows.is_empty());
    invalid(&declaration("", "", ""));
}

#[test]
fn cycles_and_self_edges_remain_declarations_while_duplicates_and_dangling_edges_fail() {
    let nodes = format!("{NODE}| catalog | 索引 | 模块 | src/catalog | 保存资料 |\n");
    let edges = "| query | catalog | 依赖 | 查询 |\n| catalog | query | 依赖 | 返回 |\n| catalog | catalog | 数据流 | 自环 |\n";
    let value = architecture(&declaration(&nodes, edges, ""));
    assert_eq!(value.status, "ready");
    assert_eq!(value.relations.len(), 3);
    for extra in [
        "| query | catalog | 依赖 | 另一种说明 |\n",
        "| query | unknown | 数据流 | 结果 |\n",
    ] {
        invalid(&declaration(&nodes, &format!("{edges}{extra}"), ""));
    }
    invalid(&declaration(
        NODE,
        "| query | query | 调用 | 未支持类型 |\n",
        "",
    ));
}

#[test]
fn static_steps_sort_and_repeat_nodes_without_creating_implicit_edges() {
    let steps = "| search | 查询 | 2 | query | 返回 |\n| search | 查询 | 1 | query | 接收 |\n";
    let value = architecture(&declaration(NODE, "", steps));
    assert_eq!(value.status, "ready");
    assert!(value.relations.is_empty());
    assert_eq!(
        value.flows[0]
            .steps
            .iter()
            .map(|step| step.order)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(value.flows[0].steps[0].source.line > value.flows[0].steps[1].source.line);
    for broken in [
        steps.replace("| 2 |", "| 1 |"),
        steps.replace("| 2 |", "| 3 |"),
        steps.replacen("查询", "另一流程名", 1),
        steps.replace("| 2 | query", "| 2 | missing"),
        steps.replace("| 1 |", "| 0 |"),
        steps.replace("| 1 |", "| 01 |"),
        steps.replace("| 1 |", "| +1 |"),
        steps.replace("| 1 |", "| 4294967296 |"),
    ] {
        invalid(&declaration(NODE, "", &broken));
    }
}

#[test]
fn ordinary_text_ids_and_paths_are_never_silently_markdown_cleaned() {
    let nodes = "| query__raw | **检索** `接口` [链接](https://example.invalid) | 接口 | src/中文目录 | 原文 \\| 保留 <b>说明</b> |\n";
    let value = architecture(&declaration(nodes, "", ""));
    assert_eq!(value.status, "ready");
    assert_eq!(value.nodes[0].id, "query__raw");
    assert_eq!(
        value.nodes[0].label,
        "**检索** `接口` [链接](https://example.invalid)"
    );
    assert_eq!(value.nodes[0].description, "原文 \\| 保留 <b>说明</b>");
    assert_eq!(value.nodes[0].directory.as_deref(), Some("src/中文目录"));
    invalid(&declaration(
        &NODE.replace("| query |", "| **query** |"),
        "",
        "",
    ));
}

#[test]
fn directory_bindings_only_accept_visible_canonical_relative_paths() {
    for directory in [
        "/tmp/repo",
        "../escape",
        "src/../other",
        "src//lib",
        "src/",
        ".git",
        "src/.hidden",
        "C:/repo",
        "src\\lib",
        "src/%2e%2e",
        "src/\u{200b}hidden",
        "src/\0bad",
        "src/e\u{301}",
    ] {
        invalid(&declaration(&NODE.replace("src/search", directory), "", ""));
    }
    for directory in [
        "src/中文目录",
        "source files/search",
        "unobserved/deep/module",
    ] {
        let value = architecture(&declaration(&NODE.replace("src/search", directory), "", ""));
        assert_eq!(value.status, "ready");
        assert_eq!(value.nodes[0].directory.as_deref(), Some(directory));
    }
}

#[test]
fn identifiers_are_bounded_ascii_and_unique_and_all_named_fields_are_required() {
    let long_id = format!("a{}", "_".repeat(47));
    assert_eq!(
        architecture(&declaration(&NODE.replace("query", &long_id), "", "")).status,
        "ready"
    );
    for id in [
        "",
        "Query",
        "1query",
        "查询",
        "query/name",
        "query.name",
        "query name",
        &format!("{long_id}a"),
    ] {
        invalid(&declaration(
            &NODE.replace("| query |", &format!("| {id} |")),
            "",
            "",
        ));
    }
    invalid(&declaration(&format!("{NODE}{NODE}"), "", ""));
    for old in ["检索接口", "接收查询", "src/search"] {
        invalid(&declaration(&NODE.replace(old, ""), "", ""));
    }
    invalid(&declaration(
        &NODE.replace("接口 | src", "未知 | src"),
        "",
        "",
    ));
    invalid(&declaration(NODE, "| query | query | 依赖 | |\n", ""));
    invalid(&declaration(NODE, "", "| search | 查询 | 1 | query | |\n"));
    invalid(&declaration(NODE, "", "| search | | 1 | query | 接收 |\n"));
}
