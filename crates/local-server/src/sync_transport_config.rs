//! 只核对 Headless 0.0.14 的本地配置 JSON；不证明认证、文件存在或同步完成。
//! 无 FS/进程/环境能力；实际根身份与本次输出来源由驱动器（`sync_plane::driver`）独立保证。
//! 对外经 `sync_plane` 以 `Headless*` 别名重导出：驱动器每轮 InspectConfig 之后按审批核对
//! 模式、排除项与五类附件，核对不过就不 RunOnce。

use serde::{
    Deserialize, Deserializer,
    de::{IgnoredAny, SeqAccess, Visitor},
};
use std::{fmt, marker::PhantomData};

const INPUT_BYTES: usize = 64 * 1024;
const TEXT_BYTES: usize = 256;
const PATH_BYTES: usize = 4096;
const CONFIG_DIR_BYTES: usize = 64;

/// 附件类型闭集；空集合允许仅批准 Markdown。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String")]
pub enum FileType {
    /// headless `image` 类。
    Image,
    /// headless `audio` 类。
    Audio,
    /// headless `video` 类。
    Video,
    /// headless `pdf` 类。
    Pdf,
    /// headless `unsupported` 类（zip、txt、json 等都归这里）。
    Unsupported,
}

impl TryFrom<String> for FileType {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "image" => Ok(Self::Image),
            "audio" => Ok(Self::Audio),
            "video" => Ok(Self::Video),
            "pdf" => Ok(Self::Pdf),
            "unsupported" => Ok(Self::Unsupported),
            _ => Err("附件类型不在固定闭集中"),
        }
    }
}

/// 驱动器给出的明确批准值；不含凭证，不自动补默认附件类型。
///
/// `sync_mode` 是本轮应处的模式（`mirror-remote`／`bidirectional`），
/// `excluded_folders` 是审批文档里的唯一母本（已排序；ADR-025 D10），两者都逐项相等才算一致。
pub struct Expected<'a> {
    /// 审批绑定的远端 Vault 身份（headless `vaultId`）。
    pub remote_vault_id: &'a str,
    /// payload 视图内的真实镜像根（headless `vaultPath`）。
    pub mirror_path: &'a str,
    /// 审批的设备名（headless `deviceName`）。
    pub device_name: &'a str,
    /// 配置目录名（headless `configDir`，形如 `.obsidian`）。
    pub config_dir: &'a str,
    /// 批准的附件类别；空集合即只批准 Markdown。
    pub file_types: &'a [FileType],
    /// 本轮应处的模式字面量。
    pub sync_mode: &'a str,
    /// 审批母本里的排除文件夹，必须已排序。
    pub excluded_folders: &'a [String],
}

/// 仅代表本次 JSON 与批准配置相同；没有远端、账号或文件系统权限。
#[derive(Debug, PartialEq, Eq)]
pub struct LocalConfigMatch;

/// 固定分类不携带原文、路径或 serde 错误，避免误回显秘密输入。
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// 输出超过 64 KiB。
    InputTooLarge,
    /// 不是封闭的命名字段对象。
    Malformed,
    /// 字段形状合法但取值越界。
    InvalidValue,
    /// 调用方给的批准值本身无效。
    InvalidExpected,
    /// 与批准值不一致。
    Mismatch,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputTooLarge => "Headless 配置输出超过字节预算",
            Self::Malformed => "Headless 配置输出不符合封闭 JSON 格式",
            Self::InvalidValue => "Headless 配置字段无效或超过预算",
            Self::InvalidExpected => "Headless 批准配置无效或超过预算",
            Self::Mismatch => "Headless 本地配置与批准值不一致",
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Config {
    vault_id: String,
    vault_name: String,
    vault_path: String,
    sync_mode: String,
    conflict_strategy: String,
    device_name: String,
    config_dir: String,
    file_types: Limited<FileType, 5>,
    #[serde(rename = "configs")]
    _configs: Limited<String, 0>,
    #[serde(rename = "excludedFolders")]
    excluded_folders: Limited<String, 64>,
}

// 不采用 JSON 的 size_hint 扩容；附件最多五项，禁止设置的数组不分配元素。
struct Limited<T, const MAX: usize>(Vec<T>);
impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for Limited<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Array<T, const MAX: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for Array<T, MAX> {
            type Value = Limited<T, MAX>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("固定数量上限的 JSON 数组")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::with_capacity(MAX);
                while values.len() < MAX {
                    let Some(value) = seq.next_element()? else {
                        return Ok(Limited(values));
                    };
                    values.push(value);
                }
                if seq.next_element::<IgnoredAny>()?.is_some() {
                    return Err(serde::de::Error::custom("数组超过固定数量上限"));
                }
                Ok(Limited(values))
            }
        }
        deserializer.deserialize_seq(Array::<T, MAX>(PhantomData))
    }
}

/// 只进行有界纯数据核对；不把成功结果解释为在线、已登录或已送达。
///
/// # Errors
/// 批准值无效返回 `InvalidExpected`；输出超预算、格式不封闭、取值越界或与批准不一致
/// 各返回对应固定分类，不带原文。
pub fn verify(input: &[u8], expected: &Expected<'_>) -> Result<LocalConfigMatch, ConfigError> {
    if !text(expected.remote_vault_id, TEXT_BYTES)
        || !absolute_path(expected.mirror_path)
        || !text(expected.device_name, TEXT_BYTES)
        || !config_directory(expected.config_dir)
        || !types_valid(expected.file_types)
        || !matches!(expected.sync_mode, "mirror-remote" | "bidirectional")
        || expected.excluded_folders.len() > 64
        || expected
            .excluded_folders
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(ConfigError::InvalidExpected);
    }
    if input.len() > INPUT_BYTES {
        return Err(ConfigError::InputTooLarge);
    }
    // serde struct 默认还可接受位置数组；官方 JSON 必须是命名字段对象。
    let first = input
        .iter()
        .copied()
        .find(|byte| !matches!(*byte, b' ' | b'\n' | b'\r' | b'\t'));
    if first != Some(b'{') {
        return Err(ConfigError::Malformed);
    }
    // from_slice 保留默认递归限制，拒绝无效 UTF8、重复/未知字段及尾部另一对象。
    let config: Config = serde_json::from_slice(input).map_err(|_| ConfigError::Malformed)?;
    if !text(&config.vault_id, TEXT_BYTES)
        || !text(&config.vault_name, TEXT_BYTES)
        || !absolute_path(&config.vault_path)
        || !text(&config.device_name, TEXT_BYTES)
        || !config_directory(&config.config_dir)
        || !text(&config.sync_mode, 32)
        || !text(&config.conflict_strategy, 16)
        || !types_valid(&config.file_types.0)
    {
        return Err(ConfigError::InvalidValue);
    }
    if config.vault_id != expected.remote_vault_id
        || config.vault_path != expected.mirror_path
        || config.device_name != expected.device_name
        || config.config_dir != expected.config_dir
        || config.sync_mode != expected.sync_mode
        || config.conflict_strategy != "conflict"
        || config.file_types.0.len() != expected.file_types.len()
        || config
            .file_types
            .0
            .iter()
            .any(|kind| !expected.file_types.contains(kind))
        || !excluded_match(&config.excluded_folders.0, expected.excluded_folders)
    {
        return Err(ConfigError::Mismatch);
    }
    Ok(LocalConfigMatch)
}

fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn absolute_path(value: &str) -> bool {
    text(value, PATH_BYTES)
        && value.starts_with('/')
        && !value.contains('\\')
        && value[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn config_directory(value: &str) -> bool {
    text(value, CONFIG_DIR_BYTES)
        && value.starts_with('.')
        && value != "."
        && value != ".."
        && !value.contains(['/', '\\'])
}

/// headless 报的排除项与审批母本必须是同一集合（去重后排序比较）；多一项少一项都不一致。
fn excluded_match(reported: &[String], expected: &[String]) -> bool {
    let mut reported: Vec<&str> = reported.iter().map(String::as_str).collect();
    reported.sort_unstable();
    reported.dedup();
    reported
        .iter()
        .copied()
        .eq(expected.iter().map(String::as_str))
}

fn types_valid(types: &[FileType]) -> bool {
    types.len() <= 5
        && types
            .iter()
            .enumerate()
            .all(|(index, kind)| !types[..index].contains(kind))
}

#[cfg(test)]
#[path = "../tests/sync_transport_private/config.rs"]
mod tests;
