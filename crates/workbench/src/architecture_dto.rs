//! 有来源的设计声明；消费者只渲染普通文本，不能据此声称已实现或正在运行。

use serde::Serialize;

use crate::SourceLocation;

/// 当前项目的可选架构声明，与实际目录观测分开。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectArchitecture {
    /// `ready`、`unavailable` 或 `invalid`；无效声明原子清空所有图数据。
    pub status: String,
    /// 固定声明文件的第一行；缺少文档时没有来源。
    pub source: Option<SourceLocation>,
    /// 按 ID 排序的声明节点。
    pub nodes: Vec<ArchitectureNode>,
    /// 按类型、起点、终点排序的声明关系，不是实测依赖。
    pub relations: Vec<ArchitectureRelation>,
    /// 按 ID 排序的静态设计路径，不是运行事实或事件回放。
    pub flows: Vec<ArchitectureFlow>,
    /// 声明缺失、错误或超过预算的原因。
    pub diagnostics: Vec<String>,
}

/// 一个模块、接口、外部主体或数据域的声明。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ArchitectureNode {
    /// 文档内唯一的受限 ASCII 标识。
    pub id: String,
    /// 人写名称，作为普通文本展示。
    pub label: String,
    /// `module`、`interface`、`external` 或 `data`。
    pub kind: String,
    /// 可见仓库相对目录；其存在性由独立目录证据判断。
    pub directory: Option<String>,
    /// 原文职责，不表达实现、部署或可用状态。
    pub description: String,
    /// 当前项目声明文件中的精确行号。
    pub source: SourceLocation,
}

/// 人写依赖或数据流声明；允许自环和循环，不伪造拓扑顺序。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ArchitectureRelation {
    /// 已声明的起点节点 ID。
    pub from: String,
    /// 已声明的终点节点 ID。
    pub to: String,
    /// `dependency` 或 `data_flow`。
    pub kind: String,
    /// 原文说明，作为普通文本展示。
    pub label: String,
    /// 关系声明所在行。
    pub source: SourceLocation,
}

/// 设计上的一次任务路径；相邻步骤不隐含调用或依赖边。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ArchitectureFlow {
    /// 文档内唯一的受限 ASCII 流程标识。
    pub id: String,
    /// 每一步共同声明的流程名称。
    pub label: String,
    /// 从 1 开始连续排序的步骤；可以重复引用同一节点。
    pub steps: Vec<ArchitectureStep>,
}

/// 单个静态设计步骤，不包含运行状态、进度或时间。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ArchitectureStep {
    /// 流程内唯一的连续正整数，最多 32。
    pub order: u32,
    /// 已声明的节点 ID。
    pub node: String,
    /// 原文步骤说明，作为普通文本展示。
    pub description: String,
    /// 步骤声明所在行。
    pub source: SourceLocation,
}
