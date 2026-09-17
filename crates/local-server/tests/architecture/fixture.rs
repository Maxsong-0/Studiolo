//! 仅向虚构临时 Vault 写入声明；复用既有 HTTP/目录夹具，不启动真实服务。

use std::fs;

use serde_json::{Value, json};

use crate::fixture::{Fixture, PROJECT};

pub const SOURCE: &str = "项目/虚构/01-框架/项目运作.md";
pub const ARCHITECTURE: &str = "# 项目运作\n\n> 人工设计声明，不表示实现或部署完成。\n\n## 节点\n| ID | 名称 | 类型 | 目录 | 职责 |\n| --- | --- | --- | --- | --- |\n| api | 查询接口 | 接口 | src/api | 接收请求 |\n| model | 只读模型 | 模块 | src/model | <script>设计文字</script> |\n| store | 内容存储 | 数据 | - | 保留人工文档 |\n| remote | 外部系统 | 外部 | - | 接收外部输入 |\n\n## 关系\n| 起点 | 终点 | 类型 | 说明 |\n| --- | --- | --- | --- |\n| api | model | 依赖 | 查询当前代次 |\n| model | store | 数据流 | 文档进入模型 |\n| api | remote | 依赖 | 声明的外部依赖 |\n\n## 任务路径\n| 流程ID | 流程名称 | 步骤 | 节点ID | 说明 |\n| --- | --- | --- | --- | --- |\n| read | 读取文档 | 1 | api | 接收查询 |\n| read | 读取文档 | 2 | model | 派生文档视图 |\n| read | 读取文档 | 3 | store | 关联文档来源 |\n";

pub fn write(fixture: &Fixture, project: &str, content: &str) {
    let parent = fixture.vault.path().join(project).join("01-框架");
    fs::create_dir_all(&parent).unwrap();
    fs::write(parent.join("项目运作.md"), content).unwrap();
}

pub fn prepared() -> Fixture {
    let fixture = Fixture::new();
    write(&fixture, PROJECT, ARCHITECTURE);
    fixture
}

pub fn node<'a>(architecture: &'a Value, id: &str) -> &'a Value {
    architecture["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == id)
        .unwrap()
}

pub fn source(value: &Value, line: u32) {
    assert_eq!(value, &json!({ "path": SOURCE, "line": line }));
}

pub fn no_graph(value: &Value, status: &str) {
    assert_eq!(value["status"], status);
    for field in ["nodes", "relations", "flows"] {
        assert!(value[field].as_array().unwrap().is_empty(), "{field}");
    }
    assert!(!value["diagnostics"].as_array().unwrap().is_empty());
}
