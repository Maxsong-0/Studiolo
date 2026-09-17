//! 正边界与越界均经公开读模型验证，预算不能截断后发布部分声明。

#[path = "support/architecture.rs"]
mod support;

use support::*;

#[test]
fn node_budget_accepts_64_and_rejects_the_65th_atomically() {
    let nodes = (0..64)
        .map(|index| format!("| n{index} | 节点 | 模块 | - | 职责 |\n"))
        .collect::<String>();
    assert_eq!(architecture(&declaration(&nodes, "", "")).nodes.len(), 64);
    invalid(&declaration(
        &format!("{nodes}| extra | 额外 | 外部 | - | 超额 |\n"),
        "",
        "",
    ));
}

#[test]
fn relation_budget_accepts_128_unique_edges_and_rejects_the_129th() {
    let nodes = (0..16)
        .map(|index| format!("| n{index} | 节点 | 模块 | - | 职责 |\n"))
        .collect::<String>();
    let edges = (0..128)
        .map(|index| format!("| n{} | n{} | 依赖 | 说明 |\n", index / 16, index % 16))
        .collect::<String>();
    assert_eq!(
        architecture(&declaration(&nodes, &edges, ""))
            .relations
            .len(),
        128
    );
    invalid(&declaration(
        &nodes,
        &format!("{edges}| n8 | n0 | 依赖 | 超额 |\n"),
        "",
    ));
}

#[test]
fn flow_and_step_budgets_accept_eight_by_32_and_reject_either_extra_dimension() {
    let flows = (0..8)
        .flat_map(|flow| {
            (1..=32).map(move |step| format!("| f{flow} | 流程 | {step} | query | 说明 |\n"))
        })
        .collect::<String>();
    let value = architecture(&declaration(NODE, "", &flows));
    assert_eq!(value.status, "ready");
    assert_eq!(value.flows.len(), 8);
    assert!(value.flows.iter().all(|flow| flow.steps.len() == 32));
    invalid(&declaration(
        NODE,
        "",
        &format!("{flows}| f8 | 流程 | 1 | query | 超额 |\n"),
    ));
    invalid(&declaration(
        NODE,
        "",
        &format!("{flows}| f0 | 流程 | 33 | query | 超额 |\n"),
    ));
}

#[test]
fn utf8_field_budgets_are_bytes_and_preserve_the_exact_positive_boundary() {
    let name = format!("{}a", "中".repeat(85));
    let description = format!("{}a", "中".repeat(341));
    let directory = "d".repeat(1024);
    let nodes = format!("| query | {name} | 模块 | {directory} | {description} |\n");
    let edges = format!("| query | query | 依赖 | {name} |\n");
    let flows = format!("| flow | {name} | 1 | query | {description} |\n");
    let value = architecture(&declaration(&nodes, &edges, &flows));
    assert_eq!(value.status, "ready");
    assert_eq!(value.nodes[0].label.len(), 256);
    assert_eq!(value.nodes[0].description.len(), 1024);
    assert_eq!(value.nodes[0].directory.as_ref().unwrap().len(), 1024);
    invalid(&declaration(
        &nodes.replace(&name, &format!("{name}a")),
        &edges,
        &flows,
    ));
    invalid(&declaration(
        &nodes.replace(&description, &format!("{description}a")),
        &edges,
        &flows,
    ));
    invalid(&declaration(
        &nodes.replace(&directory, &format!("{directory}a")),
        &edges,
        &flows,
    ));
    invalid(&declaration(
        &nodes,
        &edges.replace(&name, &format!("{name}a")),
        &flows,
    ));
    invalid(&declaration(
        &nodes,
        &edges,
        &flows.replace(&name, &format!("{name}a")),
    ));
    invalid(&declaration(
        &nodes,
        &edges,
        &flows.replace(&description, &format!("{description}a")),
    ));
}

#[test]
fn whole_document_budget_includes_prose_and_excluded_content() {
    let base = declaration(NODE, "", "");
    let exactly = format!("{base}{}", " ".repeat(64 * 1024 - base.len()));
    assert_eq!(exactly.len(), 64 * 1024);
    assert_eq!(architecture(&exactly).status, "ready");
    invalid(&format!("{exactly} "));
    invalid(&format!("<!--{}-->\n{base}", "中".repeat(22_000)));
}

#[test]
fn invalid_last_step_clears_already_parsed_nodes_relations_and_flows() {
    let edges = "| query | query | 数据流 | 结果 |\n";
    let steps = "| flow | 流程 | 1 | query | 接收 |\n| flow | 流程 | 2 | query | 返回 |\n| flow | 流程 | 3 | absent | 缺节点 |\n";
    invalid(&declaration(NODE, edges, steps));
}
