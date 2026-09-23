//! 校验共用的值类型：问题、扩大项、校验报告与 effective 上下文。

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Diff, model::PolicyKind};

/// 一条校验问题：位置与原因（不回显正文）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// 字段路径或行号。
    pub path: String,
    /// 原因。
    pub message: String,
}

impl Problem {
    pub(crate) fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// 一条权限扩大项（或收窄项）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Expansion {
    /// 字段路径或行号。
    pub path: String,
    /// 改前。
    pub before: Option<String>,
    /// 改后。
    pub after: Option<String>,
    /// 为什么算扩大。
    pub note: String,
}

/// 校验报告：结构问题、diff、扩大项、收窄项与校验时的 effective 基线。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// 是否可发布。
    pub ok: bool,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 是否为撤销（retire）该 subject。
    pub retire: bool,
    /// 结构问题；非空即不可发布。
    pub problems: Vec<Problem>,
    /// 与当前 effective 的差异。
    pub diff: Diff,
    /// 权限扩大项（界面标红）。
    pub expansions: Vec<Expansion>,
    /// 权限收窄项。
    pub narrowings: Vec<Expansion>,
    /// 校验所基于的 effective 哈希。
    pub base_effective_hash: String,
    /// 该 subject 当前生效版本（无则为空）。
    pub current_version: Option<u64>,
    /// 规范化正文的 sha256（撤销时为空串的 sha256）。
    pub body_sha256: String,
    #[serde(skip)]
    pub(crate) canonical: Option<String>,
}

/// 校验所需的 effective 上下文（在同一事务里读出）。
pub(crate) struct Context {
    /// 该 subject 当前版本与正文。
    pub current: Option<(u64, String)>,
    /// 当前 effective 哈希。
    pub effective_hash: String,
    /// 已发布的 skill 名。
    pub skills: BTreeSet<String>,
    /// skill → 硬绑定它的 AI 类型。
    pub bindings: BTreeMap<String, Vec<String>>,
}
