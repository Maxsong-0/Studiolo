//! 项目运作图首片的纯数据契约；所有标签均是待转义的普通文本。

use serde::Serialize;

use crate::SourceLocation;

/// 当前项目的目录与职责声明，不代表功能已经实现或部署。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectOperations {
    /// 已识别项目的逻辑根路径。
    pub project: String,
    /// `ready`、`unavailable` 或 `invalid`；缺资料不得当作检查通过。
    pub declaration_status: String,
    /// 当前声明块位置；无块时可指向对应文档或章节。
    pub declaration_source: Option<SourceLocation>,
    /// 按路径排序的目录，含由路径推得的隐含父目录。
    pub directories: Vec<DirectoryDeclaration>,
    /// 明确展开的父层；空串是仓库根，不比较未展开目录的内容。
    pub comparison_parents: Vec<String>,
    /// 有界解析与不可用原因。
    pub diagnostics: Vec<String>,
}

/// 文档声明的相对目录及职责。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DirectoryDeclaration {
    /// 无尾随分隔符的可见相对目录。
    pub path: String,
    /// 原文职责或目录名；消费者必须作为文本渲染。
    pub label: String,
    /// 明确声明所在行；隐含父目录没有独立职责来源。
    pub source: Option<SourceLocation>,
}

/// 调用方提供的一个父目录完整观测；本模块不进行文件系统访问。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DirectoryObservation {
    /// 父目录相对路径，空串表示仓库根。
    pub path: String,
    /// `ready` 是完整列举，`missing` 是确认不存在，其他状态不可验证。
    pub status: String,
    /// 直属可见子目录的完整相对路径；失败或不完整不得标 `ready`。
    pub children: Vec<String>,
}

/// 一条目录存在性或未文档化结果，不表达运行时能力状态。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DirectoryDrift {
    /// 仓库相对目录路径。
    pub path: String,
    /// `present`、`missing`、`undocumented` 或 `unverified`。
    pub status: String,
    /// 对应声明行；隐含目录和未文档化目录没有独立来源。
    pub source: Option<SourceLocation>,
}
