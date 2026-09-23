//! 正规化 frontmatter 的纯数据类型；缓存内容仍是非权威声明。

use std::collections::BTreeMap;

/// 某个正规化 frontmatter 键可接受的一组拼写。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyAliasGroup {
    /// 暴露给调用方的英文规范键。
    pub canonical: &'static str,
    /// 读取时接受的其他拼写。
    pub aliases: &'static [&'static str],
}

/// M0 支持的八组 frontmatter 拼写。
pub const FRONTMATTER_KEY_ALIASES: &[KeyAliasGroup] = &[
    KeyAliasGroup {
        canonical: "status",
        aliases: &["状态"],
    },
    KeyAliasGroup {
        canonical: "date",
        aliases: &["日期"],
    },
    KeyAliasGroup {
        canonical: "project",
        aliases: &["项目"],
    },
    KeyAliasGroup {
        canonical: "type",
        aliases: &["类型"],
    },
    KeyAliasGroup {
        canonical: "id",
        aliases: &["编号"],
    },
    KeyAliasGroup {
        canonical: "severity",
        aliases: &["严重度", "严重级"],
    },
    KeyAliasGroup {
        canonical: "updated",
        aliases: &["更新", "更新日期"],
    },
    KeyAliasGroup {
        canonical: "created",
        aliases: &["创建"],
    },
];

/// 独立的 BUG 日期别名，不得并入 `created` 或 `updated`。
pub const FRONTMATTER_SPECIAL_KEY_ALIASES: &[KeyAliasGroup] = &[
    KeyAliasGroup {
        canonical: "discovered_at",
        aliases: &["发现日期"],
    },
    KeyAliasGroup {
        canonical: "fixed_at",
        aliases: &["修复日期"],
    },
];

/// 读侧由 YAML 得到的值。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub enum FrontmatterValue {
    /// YAML 空值。
    Null,
    /// YAML 布尔值。
    Bool(bool),
    /// YAML 数字，存为展示用字符串：由解析出的数值重新格式化，不保留源拼写。
    Number(String),
    /// YAML 字符串标量。
    Text(String),
    /// YAML 序列。
    List(Vec<FrontmatterValue>),
    /// YAML 映射。
    Mapping(BTreeMap<String, FrontmatterValue>),
    /// `status` 的闭集正规化结果。
    Status(NormalizedStatus),
}

/// 只供派生视图使用的闭集状态。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub enum NormalizedStatus {
    /// 已采纳.
    Adopted,
    /// 已修复.
    Fixed,
    /// 已完成或已上线。
    Done,
    /// 正在进行中。
    InProgress,
    /// 仍有后续动作待办。
    Pending,
    /// 已批准。
    Approved,
    /// 已被取代。
    Superseded,
    /// 已废弃。
    Deprecated,
    /// 草稿或构想。
    Draft,
    /// 不在闭集内；保留原文。
    Unknown(String),
}

/// 一个原始顶层 YAML 键及其正规化解释。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterEntry {
    /// 原始键拼写（去掉包裹的引号）。
    pub raw_key: String,
    /// 顶层冒号之后的原始 YAML 表示。
    pub raw_value: String,
    /// 读侧规范键。
    pub canonical_key: String,
    /// 解析并正规化后的值。
    pub value: FrontmatterValue,
}

/// 非致命的 frontmatter 诊断。
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub enum FrontmatterWarning {
    /// 同一规范键出现了多种拼写（规范键与别名、多个别名或仅大小写不同的规范键并存）；
    /// 规范拼写优先，同优先级取源顺序中的第一个。
    DuplicateAliasGroup {
        /// 拼写发生冲突的规范键。
        canonical: String,
        /// 笔记中出现的原始拼写，按字典序排列。
        raw_keys: Vec<String>,
    },
    /// YAML 无法反序列化。
    MalformedYaml {
        /// `serde_saphyr` 渲染的错误文案：含行列位置与出错处附近的 YAML 源码片段（即笔记
        /// 内容），不含宿主路径。
        message: String,
    },
    /// 有起始分隔线却没有结束分隔线。
    Unclosed,
}

impl FrontmatterWarning {
    /// 针对具体键的警告返回受影响的规范键，其余返回 `None`。
    #[must_use]
    pub fn canonical_key(&self) -> Option<&str> {
        match self {
            Self::DuplicateAliasGroup { canonical, .. } => Some(canonical),
            Self::MalformedYaml { .. } | Self::Unclosed => None,
        }
    }
}

/// 为读取而正规化、同时保留源文本的 frontmatter。
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedFrontmatter {
    /// 两条分隔线之间的 YAML 原文；未闭合时为起始分隔线之后的全部文本。
    pub raw: String,
    /// 选定的值；英文规范拼写优先于别名。
    pub canonical: BTreeMap<String, FrontmatterValue>,
    /// 按源顺序排列的全部原始顶层字段。
    pub entries: Vec<FrontmatterEntry>,
    /// 解析时被正规化的别名拼写，每项为（原始拼写, 规范键）。
    pub alias_hits: Vec<(String, String)>,
    /// 非致命诊断。
    pub warnings: Vec<FrontmatterWarning>,
}

impl NormalizedFrontmatter {
    /// 按规范键查找选定的值。
    #[must_use]
    pub fn get(&self, canonical_key: &str) -> Option<&FrontmatterValue> {
        self.canonical.get(canonical_key)
    }
}
