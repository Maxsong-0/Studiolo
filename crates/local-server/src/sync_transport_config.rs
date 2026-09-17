//! 只核对 Headless 0.0.14 的本地配置 JSON；不证明认证、文件存在或同步完成。
//! 无 FS/进程/环境能力；实际根身份与本次输出来源由未来 owner 独立保证。

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
pub(crate) enum FileType {
    Image,
    Audio,
    Video,
    Pdf,
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

/// 未来 owner 给出的明确批准值；不含凭证，不自动补默认附件类型。
pub(crate) struct Expected<'a> {
    pub(crate) remote_vault_id: &'a str,
    pub(crate) mirror_path: &'a str,
    pub(crate) device_name: &'a str,
    pub(crate) config_dir: &'a str,
    pub(crate) file_types: &'a [FileType],
}

/// 仅代表本次 JSON 与批准配置相同；没有远端、账号或文件系统权限。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LocalConfigMatch;

/// 固定分类不携带原文、路径或 serde 错误，避免误回显秘密输入。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConfigError {
    InputTooLarge,
    Malformed,
    InvalidValue,
    InvalidExpected,
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
    _excluded_folders: Limited<String, 0>,
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
pub(crate) fn verify(
    input: &[u8],
    expected: &Expected<'_>,
) -> Result<LocalConfigMatch, ConfigError> {
    if !text(expected.remote_vault_id, TEXT_BYTES)
        || !absolute_path(expected.mirror_path)
        || !text(expected.device_name, TEXT_BYTES)
        || !config_directory(expected.config_dir)
        || !types_valid(expected.file_types)
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
        || config.sync_mode != "bidirectional"
        || config.conflict_strategy != "conflict"
        || config.file_types.0.len() != expected.file_types.len()
        || config
            .file_types
            .0
            .iter()
            .any(|kind| !expected.file_types.contains(kind))
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
