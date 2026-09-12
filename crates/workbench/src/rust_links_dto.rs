//! 同批输入中的受限词法关联；没有编译器解析或执行保证。

use serde::Serialize;

use crate::RustCallGraph;

/// 原调用图与关联来自同一组已验证语法树。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustCallAnalysis {
    /// 保持 T-044 语义的词法上下文和调用位置。
    pub graph: RustCallGraph,
    /// 每个调用恰有一条关联结果。
    pub links: RustCallLinks,
}

/// 输入内容身份和按原调用顺序排列的结果。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustCallLinks {
    /// 当前契约版本，固定为 1。
    pub version: u32,
    /// 带域和长度前缀的完整输入 SHA-256，小写十六进制。
    pub batch_id: String,
    /// 顺序与 graph.calls 相同，不遗漏未解析调用。
    pub associations: Vec<RustCallAssociation>,
}

/// 唯一自由函数关联或明确未解析原因，不附猜测目标。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RustCallAssociation {
    /// 同批原调用 ID。
    pub call_id: String,
    /// linked 或 unresolved。
    pub status: String,
    /// linked 时为同文件 function context ID，其余为 None。
    pub target: Option<String>,
    /// unresolved 的受控原因，linked 时为 None。
    pub reason: Option<String>,
}
