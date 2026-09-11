//! 正规化 frontmatter 的纯数据类型；缓存内容仍是非权威声明。

use std::collections::BTreeMap;

/// One accepted spelling group for a normalized frontmatter key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyAliasGroup {
    /// English key exposed to callers.
    pub canonical: &'static str,
    /// Alternate spellings accepted on read.
    pub aliases: &'static [&'static str],
}

/// The eight frontmatter spelling groups supported by M0.
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

/// Independent BUG-date aliases that must not collapse into `created` or `updated`.
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

/// Read-side value produced from YAML.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub enum FrontmatterValue {
    /// YAML null.
    Null,
    /// YAML boolean.
    Bool(bool),
    /// YAML number retained as a display string.
    Number(String),
    /// YAML string scalar.
    Text(String),
    /// YAML sequence.
    List(Vec<FrontmatterValue>),
    /// YAML mapping.
    Mapping(BTreeMap<String, FrontmatterValue>),
    /// Closed-set normalization for `status`.
    Status(NormalizedStatus),
}

/// Closed status set used only by derived views.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub enum NormalizedStatus {
    /// 已采纳.
    Adopted,
    /// 已修复.
    Fixed,
    /// Complete or live.
    Done,
    /// Currently in progress.
    InProgress,
    /// A further action is pending.
    Pending,
    /// Approved.
    Approved,
    /// Superseded.
    Superseded,
    /// Deprecated.
    Deprecated,
    /// Draft or concept.
    Draft,
    /// Outside the closed set; original text retained.
    Unknown(String),
}

/// One original top-level YAML key and its normalized interpretation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrontmatterEntry {
    /// Original key spelling.
    pub raw_key: String,
    /// Original YAML representation after the top-level colon.
    pub raw_value: String,
    /// Canonical read-side key.
    pub canonical_key: String,
    /// Parsed and normalized value.
    pub value: FrontmatterValue,
}

/// Non-fatal frontmatter diagnostic.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub enum FrontmatterWarning {
    /// A canonical key and aliases coexist; canonical spelling wins.
    DuplicateAliasGroup {
        /// Canonical key whose spellings collided.
        canonical: String,
        /// Original spellings found in the note.
        raw_keys: Vec<String>,
    },
    /// YAML could not be deserialized.
    MalformedYaml {
        /// Parser message without host paths or note contents.
        message: String,
    },
    /// An opening delimiter had no closing delimiter.
    Unclosed,
}

impl FrontmatterWarning {
    /// Returns the affected canonical key for key-specific warnings.
    #[must_use]
    pub fn canonical_key(&self) -> Option<&str> {
        match self {
            Self::DuplicateAliasGroup { canonical, .. } => Some(canonical),
            Self::MalformedYaml { .. } | Self::Unclosed => None,
        }
    }
}

/// Frontmatter normalized for reads while retaining source text.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedFrontmatter {
    /// Exact YAML text between delimiters.
    pub raw: String,
    /// Selected values; canonical English spelling wins over aliases.
    pub canonical: BTreeMap<String, FrontmatterValue>,
    /// Every original top-level field in source order.
    pub entries: Vec<FrontmatterEntry>,
    /// Alternate spellings normalized during parsing.
    pub alias_hits: Vec<(String, String)>,
    /// Non-fatal diagnostics.
    pub warnings: Vec<FrontmatterWarning>,
}

impl NormalizedFrontmatter {
    /// Looks up a selected value by canonical key.
    #[must_use]
    pub fn get(&self, canonical_key: &str) -> Option<&FrontmatterValue> {
        self.canonical.get(canonical_key)
    }
}
