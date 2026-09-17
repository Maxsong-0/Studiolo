//! 架构声明纯契约的共用虚构输入；各测试仅导入自己需要的部分。
#![allow(dead_code)]

use studiolo_core::VaultPath;
use studiolo_vault_index::MarkdownDocument;
use studiolo_workbench::{ProjectArchitecture, WorkbenchReadModel};

pub const PROJECT: &str = "项目/示例工坊";
pub const PATH: &str = "项目/示例工坊/01-框架/项目运作.md";
pub const NODE_HEADER: &str = "| ID | 名称 | 类型 | 目录 | 职责 |\n|---|---|---|---|---|\n";
pub const RELATION_HEADER: &str = "| 起点 | 终点 | 类型 | 说明 |\n|---|---|---|---|\n";
pub const FLOW_HEADER: &str =
    "| 流程ID | 流程名称 | 步骤 | 节点ID | 说明 |\n|---|---|---|---|---|\n";
pub const NODE: &str = "| query | 检索接口 | 接口 | src/search | 接收查询 |\n";

pub fn document(path: &str, content: &str) -> MarkdownDocument {
    MarkdownDocument::new(VaultPath::parse(path).unwrap(), content)
}

pub fn model(content: &str) -> WorkbenchReadModel {
    WorkbenchReadModel::build(&[document(PATH, content)]).unwrap()
}

pub fn architecture(content: &str) -> ProjectArchitecture {
    model(content).project_architecture(PROJECT).unwrap()
}

pub fn declaration(nodes: &str, relations: &str, flows: &str) -> String {
    format!(
        "# 设计声明\n## 节点\n{NODE_HEADER}{nodes}\n## 关系\n{RELATION_HEADER}{relations}\n## 任务路径\n{FLOW_HEADER}{flows}"
    )
}

pub fn invalid(content: &str) {
    let value = architecture(content);
    assert_eq!(value.status, "invalid", "输入应当无效：{content}");
    assert!(value.nodes.is_empty());
    assert!(value.relations.is_empty());
    assert!(value.flows.is_empty());
    let source = value.source.unwrap();
    assert_eq!(source.path, PATH);
    assert_eq!(source.line, 1);
    assert!(!value.diagnostics.is_empty());
}
