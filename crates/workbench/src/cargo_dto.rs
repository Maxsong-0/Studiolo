//! Cargo 清单的纯数据证据，不表示源码调用、特性解析或运行状态。

use serde::Serialize;

/// 由受信任采集器提供的完整清单正文；不会包含在输出图中。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoManifest {
    /// 规范的仓库相对清单路径。
    pub path: String,
    /// 本轮完整 UTF-8 正文。
    pub content: String,
}

/// 仓库清单来源，与 Vault 笔记来源分开。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ManifestSource {
    /// 仓库相对 Cargo.toml 路径，不是宿主路径或 Vault 路径。
    pub path: String,
    /// 原文一基行号。
    pub line: usize,
}

/// 输入错误和受控子集不能表达的语义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CargoDependencyErrorKind {
    /// TOML、字段、身份或输入集合损坏。
    Invalid,
    /// 合法或未知语义超出本片范围，包括预算超限。
    Unsupported,
}

/// 只含受控中文原因，不带 TOML 原始错误、URL 或正文值。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CargoDependencyError {
    /// 失败类型；上层可独立映射显示状态。
    pub kind: CargoDependencyErrorKind,
    /// 固定中文说明。
    pub message: String,
}

impl CargoDependencyError {
    pub(crate) fn invalid(message: &'static str) -> Self {
        Self {
            kind: CargoDependencyErrorKind::Invalid,
            message: message.to_owned(),
        }
    }

    pub(crate) fn unsupported(message: &'static str) -> Self {
        Self {
            kind: CargoDependencyErrorKind::Unsupported,
            message: message.to_owned(),
        }
    }
}

/// 显式成员之间的清单依赖；外部条目只计数，不输出细节。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CargoDependencyGraph {
    /// 按规范目录 ID 排序的包。
    pub packages: Vec<CargoPackage>,
    /// 按起点、终点、种类、目标条件、别名排序的内部依赖。
    pub dependencies: Vec<CargoDependency>,
    /// 非 path 依赖的声明次数，包含继承条目，不是去重包数。
    pub external_dependencies: u32,
}

/// 根 package 或 workspace.members 明确列出的 package。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CargoPackage {
    /// 规范目录；根包为 `.`。
    pub id: String,
    /// 原文 package.name，不依据目录名猜测。
    pub name: String,
    /// package.name 的原文位置。
    pub source: ManifestSource,
}

/// 一项显式本地依赖；保留所有条件，不计算是否启用。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CargoDependency {
    /// 依赖方包目录 ID。
    pub from: String,
    /// 被依赖包目录 ID。
    pub to: String,
    /// 调用方清单中的依赖别名。
    pub alias: String,
    /// `normal`、`dev` 或 `build`。
    pub kind: String,
    /// 原文 target 条件；不判断真假。
    pub target: Option<String>,
    /// 清单 optional 标记，默认 false。
    pub optional: bool,
    /// 调用方依赖 alias 所在行。
    pub source: ManifestSource,
    /// workspace.dependencies 定义位置；直接依赖为 None。
    pub inherited_from: Option<ManifestSource>,
}
