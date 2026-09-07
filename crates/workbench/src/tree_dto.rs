//! 文件树的公开值类型；只携带逻辑路径，不暴露宿主路径或文件句柄。

use serde::Serialize;
use studiolo_core::VaultPath;

/// 扫描批次中的节点类型；序列化为稳定的小写下划线标识。
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TreeKind {
    /// 可见目录，包括空目录及只有附件的目录。
    Directory,
    /// 扩展名为 `.md` 的文件，不区分扩展名 ASCII 大小写。
    Markdown,
    /// 非 Markdown 的普通文件；文件树不读取其内容。
    Attachment,
}

/// 显式提供的完整目录清单；根目录隐含存在，不作为输入条目。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TreeInput {
    /// 相对选定根目录的可见规范路径。
    pub path: VaultPath,
    /// 扫描确认的节点类型，不由前端或标题猜测。
    pub kind: TreeKind,
    /// 文件必须有字节数，目录必须没有；零字节文件合法。
    pub size_bytes: Option<u64>,
}

/// 一条直属子节点；返回值独立于当前不可变文件树。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TreeEntry {
    /// 完整逻辑路径，不截断、不转成宿主路径。
    pub path: String,
    /// 最后一段路径名称，不依赖 Markdown 标题。
    pub name: String,
    /// 节点类型。
    pub kind: TreeKind,
    /// 文件大小的十进制字符串，避免 JavaScript 整数精度损失；目录为 null。
    pub size_bytes: Option<String>,
    /// 目录的直属子节点总数，包括零；文件为 null。
    pub child_count: Option<usize>,
}

/// 同一不可变目录的有界分页结果，每页最多 200 条。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TreePage {
    /// 被查询的规范目录路径；空串表示根目录。
    pub path: String,
    /// 本页从直属子节点列表中的哪个位置开始。
    pub offset: usize,
    /// 此目录的直属子节点总数，不是递归后代数。
    pub total: usize,
    /// 目录优先，其次按完整逻辑路径稳定排序的本页节点。
    pub entries: Vec<TreeEntry>,
    /// 下一页位置；终页序列化为 null。
    pub next_offset: Option<usize>,
}

/// 构建或查询的有限错误集合，不包含源文、宿主路径或内部错误详情。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    /// 输入清单或 Markdown 批次不完整、不规范、重复或相互不一致。
    #[error("文件树输入不完整或不一致。")]
    InvalidInput,
    /// 查询不是可见规范相对路径，且不是表示根目录的空串。
    #[error("文件树路径不合法。")]
    InvalidPath,
    /// 规范路径不属于当前不可变文件树。
    #[error("文件树路径不存在。")]
    NotFound,
    /// 路径属于文件，不能按目录分页。
    #[error("文件树路径不是目录。")]
    NotDirectory,
    /// 起点超出直属子节点范围；空目录仅接受零。
    #[error("文件树分页位置超出范围。")]
    InvalidOffset,
}
