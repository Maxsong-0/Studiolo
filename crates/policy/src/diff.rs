//! desired 与 effective 的差异：JSON 母本按字段路径比对，Markdown 按行做 LCS。
//!
//! 差异只用于人审阅，不参与授权判断；扩大项的判定在 `validate`。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 差异条目类别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffOp {
    /// 新增。
    Added,
    /// 删除。
    Removed,
    /// 改变。
    Changed,
}

/// 一条差异：JSON 母本 `path` 是字段路径，Markdown 是 `old:<行>` / `new:<行>`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffEntry {
    /// 类别。
    pub op: DiffOp,
    /// 位置。
    pub path: String,
    /// 改前值（新增时为空）。
    pub before: Option<String>,
    /// 改后值（删除时为空）。
    pub after: Option<String>,
}

/// 差异集合与计数；超出上限时 `truncated` 为 true，条目只保留前一部分。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    /// `json` 或 `text`。
    pub format: String,
    /// 条目（最多 [`MAX_ENTRIES`]）。
    pub entries: Vec<DiffEntry>,
    /// 新增计数。
    pub added: usize,
    /// 删除计数。
    pub removed: usize,
    /// 改变计数。
    pub changed: usize,
    /// 条目被截断或正文太大改为整体替换。
    pub truncated: bool,
}

/// 单份差异保留的最多条目数。
pub const MAX_ENTRIES: usize = 2000;
const MAX_LCS_CELLS: usize = 4_000_000;

impl Diff {
    /// 空差异。
    #[must_use]
    pub fn empty(format: &str) -> Self {
        Self {
            format: format.to_owned(),
            entries: Vec::new(),
            added: 0,
            removed: 0,
            changed: 0,
            truncated: false,
        }
    }

    fn push(&mut self, entry: DiffEntry) {
        match entry.op {
            DiffOp::Added => self.added += 1,
            DiffOp::Removed => self.removed += 1,
            DiffOp::Changed => self.changed += 1,
        }
        if self.entries.len() < MAX_ENTRIES {
            self.entries.push(entry);
        } else {
            self.truncated = true;
        }
    }
}

fn flatten(prefix: &str, value: &Value, out: &mut BTreeMap<String, String>) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.insert(prefix.to_owned(), "{}".into());
            }
            for (key, nested) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, nested, out);
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.insert(prefix.to_owned(), "[]".into());
            }
            for (index, nested) in items.iter().enumerate() {
                flatten(&format!("{prefix}[{index}]"), nested, out);
            }
        }
        scalar => {
            out.insert(prefix.to_owned(), scalar.to_string());
        }
    }
}

/// JSON 字段级差异；`None` 表示该侧不存在。
pub(crate) fn json_diff(before: Option<&Value>, after: Option<&Value>) -> Diff {
    let mut diff = Diff::empty("json");
    let mut left = BTreeMap::new();
    let mut right = BTreeMap::new();
    if let Some(value) = before {
        flatten("", value, &mut left);
    }
    if let Some(value) = after {
        flatten("", value, &mut right);
    }
    for (path, value) in &left {
        match right.get(path) {
            None => diff.push(DiffEntry {
                op: DiffOp::Removed,
                path: path.clone(),
                before: Some(value.clone()),
                after: None,
            }),
            Some(other) if other != value => diff.push(DiffEntry {
                op: DiffOp::Changed,
                path: path.clone(),
                before: Some(value.clone()),
                after: Some(other.clone()),
            }),
            Some(_) => {}
        }
    }
    for (path, value) in &right {
        if !left.contains_key(path) {
            diff.push(DiffEntry {
                op: DiffOp::Added,
                path: path.clone(),
                before: None,
                after: Some(value.clone()),
            });
        }
    }
    diff
}

/// 行级 LCS 差异；正文过大时退化为整体替换并标记截断。
pub(crate) fn text_diff(before: &str, after: &str) -> Diff {
    let mut diff = Diff::empty("text");
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    if old.len().saturating_mul(new.len()) > MAX_LCS_CELLS {
        diff.truncated = true;
        for (index, line) in old.iter().enumerate() {
            diff.push(DiffEntry {
                op: DiffOp::Removed,
                path: format!("old:{}", index + 1),
                before: Some((*line).to_owned()),
                after: None,
            });
        }
        for (index, line) in new.iter().enumerate() {
            diff.push(DiffEntry {
                op: DiffOp::Added,
                path: format!("new:{}", index + 1),
                before: None,
                after: Some((*line).to_owned()),
            });
        }
        return diff;
    }
    // 经典 LCS 表：table[i][j] = old[i..] 与 new[j..] 的最长公共子序列长度。
    let mut table = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
        } else if j < new.len() && (i == old.len() || table[i][j + 1] >= table[i + 1][j]) {
            diff.push(DiffEntry {
                op: DiffOp::Added,
                path: format!("new:{}", j + 1),
                before: None,
                after: Some(new[j].to_owned()),
            });
            j += 1;
        } else {
            diff.push(DiffEntry {
                op: DiffOp::Removed,
                path: format!("old:{}", i + 1),
                before: Some(old[i].to_owned()),
                after: None,
            });
            i += 1;
        }
    }
    diff
}
