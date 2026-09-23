use serde::{Deserialize, Serialize};

/// 在一个不可变 Markdown 代次上做的有界字面搜索。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchQuery {
    /// 至多 256 个 Unicode 标量、1,024 个 UTF-8 字节，且不含控制字符。
    pub text: String,
    /// 精确的已知项目根身份；缺省时搜索全部已索引文档。
    pub project: Option<String>,
    /// 完整路径／正文字面搜索，或要求全部片段命中的路径查找。
    pub mode: SearchMode,
}

/// 全部调用方共用的搜索语义，包括键盘路径查找。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// 对完整路径与正文各行做不区分大小写的字面匹配。
    FullText,
    /// 按任意 Unicode 空白（含全角空格）切出的每个片段都必须出现在完整路径中。
    Path,
}

/// 一条可按源定位的搜索结果，不含宿主文件系统路径。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchHit {
    /// 被搜索代次中的逻辑文档路径。
    pub path: String,
    /// M0 选定的标题，至多 320 个 Unicode 标量；结尾的 `…` 表示已截断。
    pub title: String,
    /// 源文件行号（从 1 起）；路径命中指向第一行。
    pub line: u32,
    /// 至多 320 个 Unicode 标量，从首个匹配位置之前至多 32 个字符处开始截取。
    pub snippet: String,
    /// 匹配发生在逻辑路径还是源文本行。
    pub kind: SearchHitKind,
}

/// 字面匹配的来源。
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchHitKind {
    /// 完整文档路径。
    Path,
    /// 一行源文本。
    Content,
}

/// 一个不可变代次上确定、有界的结果批次。
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SearchResults {
    /// 按逻辑路径／源行顺序排列的前 100 条命中。
    pub hits: Vec<SearchHit>,
    /// 只有观察到第 101 条命中时才为真。
    pub truncated: bool,
}

/// 被拒绝的搜索请求，或内存匹配失败。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// 查询的大小或字符违反共享搜索接口的约束。
    #[error("搜索内容过长或包含控制字符。")]
    InvalidQuery,
    /// 项目筛选不是当前代次中的精确身份。
    #[error("项目筛选不属于当前视图。")]
    InvalidProject,
    /// 字面匹配器构造失败或内存搜索失败。
    #[error("内存搜索失败：{0}")]
    Engine(String),
}
