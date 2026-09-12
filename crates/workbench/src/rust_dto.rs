//! 未展开 Rust 语法的调用位置；不表示名字解析结果或执行轨迹。

use serde::Serialize;

/// 采集器提供的完整源码输入；正文不进入返回值。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustSourceInput {
    /// 规范的仓库相对 .rs 路径。
    pub path: String,
    /// 本轮完整 UTF-8 正文。
    pub content: String,
}

/// 无效输入与受控范围或预算以外的输入分别报告。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustSourceErrorKind {
    /// 路径、语法或必需输出标识无效。
    Invalid,
    /// 输入超出语法支持范围或资源预算。
    Unsupported,
}

/// 固定中文错误，不带解析器日志、路径值或源码。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RustSourceError {
    /// 失败类型，由适配层映射为独立状态。
    pub kind: RustSourceErrorKind,
    /// 不含原始值的中文原因。
    pub message: String,
}

impl RustSourceError {
    pub(crate) fn invalid(message: &'static str) -> Self {
        Self {
            kind: RustSourceErrorKind::Invalid,
            message: message.to_owned(),
        }
    }

    pub(crate) fn unsupported(message: &'static str) -> Self {
        Self {
            kind: RustSourceErrorKind::Unsupported,
            message: message.to_owned(),
        }
    }
}

/// 仓库相对位置；行、UTF-8 字节列均从 1 开始。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustSourceLocation {
    /// 仓库相对源码路径，与 Vault 笔记来源分开。
    pub path: String,
    /// 原文一基行号。
    pub line: u32,
    /// 原文一基 UTF-8 字节列。
    pub column: u32,
}

/// 三个扁平集合；没有递归结构或跨函数目标边。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustCallGraph {
    /// 按路径排序的完整覆盖集合。
    pub files: Vec<RustSourceCoverage>,
    /// 按路径及语法范围排序的词法容器。
    pub contexts: Vec<RustCallContext>,
    /// 按路径及语法范围排序的调用位置。
    pub calls: Vec<RustCallSite>,
}

/// 明确说明本次覆盖和未展开语法的数量。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustSourceCoverage {
    /// 本次明确读取的相对路径。
    pub path: String,
    /// 原文 UTF-8 字节数。
    pub bytes: u32,
    /// 未展开的外部和内部属性数量。
    pub attributes: u32,
    /// 其中 cfg/cfg_attr 条件属性数量，不求值。
    pub conditional_attributes: u32,
    /// 已记录位置但未展开的宏 invocation 数量。
    pub unexpanded_macros: u32,
    /// 跳过正文的宏定义数量。
    pub macro_definitions: u32,
    /// 没有内联正文、不会跟随读取的 mod 声明数量。
    pub external_modules: u32,
}

/// 最近的词法容器；包含关系不代表立即执行或先后顺序。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustCallContext {
    /// 与输入顺序无关的文件序号和字节范围标识。
    pub id: String,
    /// 同文件最近外层上下文；仅文件根为 None。
    pub parent: Option<String>,
    /// 受控词法种类，不表示执行状态。
    pub kind: String,
    /// 原标识；匿名或不可命名的容器为 None。
    pub name: Option<String>,
    /// 语法节点起点，文件根固定为 1:1。
    pub source: RustSourceLocation,
}

/// 一个调用表达式或宏 invocation；参数和接收者正文不输出。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustCallSite {
    /// 与输入顺序无关的文件序号和字节范围标识。
    pub id: String,
    /// 所属的同文件最近词法上下文 ID。
    pub context: String,
    /// path、method、expression 或 macro。
    pub kind: String,
    /// 标识路径或方法名；expression 固定为 None。
    pub name: Option<String>,
    /// 调用语法节点起点，不是已解析目标位置。
    pub source: RustSourceLocation,
}
