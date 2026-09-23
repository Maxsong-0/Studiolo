use serde::Serialize;

/// 输入 Markdown 文档中的位置，行号从 1 起。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SourceLocation {
    /// 逻辑 Vault 路径，绝不是宿主文件系统路径。
    pub path: String,
    /// 源文件行号（从 1 起）。
    pub line: u32,
}

/// 显式派生出的值及其证据；未知值两者皆无。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Sourced<T> {
    /// 已知值；证据缺失或有歧义时为 `None`。
    pub value: Option<T>,
    /// 已知值的证据位置。
    pub source: Option<SourceLocation>,
}

impl<T> Sourced<T> {
    pub(crate) fn unknown() -> Self {
        Self {
            value: None,
            source: None,
        }
    }

    pub(crate) fn known(value: T, source: SourceLocation) -> Self {
        Self {
            value: Some(value),
            source: Some(source),
        }
    }
}

/// 只从当前、可识别的项目文档派生的项目事实。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProjectSummary {
    /// 稳定的逻辑身份，等于项目根。
    pub id: String,
    /// `项目/` 下的文件夹名。
    pub name: String,
    /// 逻辑项目目录，不带结尾斜杠。
    pub root: String,
    /// 显式的生命周期声明；绝不根据活跃度推断。
    pub lifecycle: Sourced<String>,
    /// INDEX 中显式声明的当前版本。
    pub version: Sourced<String>,
    /// TASKS ① 区中未完成的任务行数。
    pub active_tasks: Sourced<u64>,
    /// 未关闭 BUG 数；任一 BUG 状态无法归类或没有可核验的状态来源时为未知。
    pub open_bugs: Sourced<u64>,
    /// roadmap 中第一个未明确标为完成的里程碑。
    pub next_milestone: Sourced<String>,
    /// `standard`、`partial` 或 `unfiled`，只描述文档结构。
    pub profile: String,
    /// INDEX 的位置（存在 INDEX 时）。
    pub source: Option<SourceLocation>,
    /// 保守解析与证据缺失提示。
    pub diagnostics: Vec<String>,
}

/// 当前、显式的用户待办动作，附源链接。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ActionItem {
    /// 由源路径、行号与动作类别组成的稳定身份。
    pub id: String,
    /// 项目根身份。
    pub project_id: String,
    /// 项目文件夹名。
    pub project_name: String,
    /// 待验收、待拍板、待重启、待外部方或待用户操作。
    pub kind: String,
    /// 从源文本派生的动作文本。
    pub title: String,
    /// 精确的证据位置。
    pub source: SourceLocation,
}

/// 可导航 Markdown 笔记的元数据。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocumentSummary {
    /// 逻辑 Vault 路径。
    pub path: String,
    /// M0 VaultIndex 选定的展示标题。
    pub title: String,
}

/// 可导航笔记未经改动的源文本。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DocumentView {
    /// 逻辑 Vault 路径。
    pub path: String,
    /// M0 VaultIndex 选定的展示标题。
    pub title: String,
    /// 原样输入的 Markdown，不是渲染后的 HTML。
    pub content: String,
}

/// 一整个输入代次的确定性视图；数据全部自持，不借用读模型。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WorkbenchSnapshot {
    /// 按逻辑根排序的项目。
    pub projects: Vec<ProjectSummary>,
    /// 按项目、源路径、源行号与类别排序的待办动作。
    pub inbox: Vec<ActionItem>,
    /// 全部输入 Markdown 文档，按逻辑路径排序。
    pub documents: Vec<DocumentSummary>,
    /// 整个代次范围的解析提示。
    pub diagnostics: Vec<String>,
}
