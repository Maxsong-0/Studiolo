//! 有界且不回显输入的参数解析。

use crate::Error;
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};
use studiolo_core::VaultPath;

/// 明确选择只读规划或一次性初始化。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// 只读规划，不产生任何持久文件。
    Plan,
    /// 仅执行调用方精确批准的计划。
    Initialize {
        /// 64 位十六进制计划摘要，不是秘密。
        expected_plan: String,
    },
}

/// 本地操作者提供的非秘密输入；每个根都在执行时重新审批。
#[derive(Clone, Debug)]
pub struct Options {
    /// 现有私有主人配置。
    pub config: PathBuf,
    /// 预先创建、空且私有的 canonical 目录。
    pub vault: PathBuf,
    /// 预先创建、空且私有的 Broker 数据目录。
    pub writer_data: PathBuf,
    /// 已批准的静态资源目录。
    pub assets: PathBuf,
    /// 操作者冻结的独立私有快照目录。
    pub source_snapshot: PathBuf,
    /// 明确选择的非隐藏逻辑路径前缀。
    pub allow: Vec<VaultPath>,
}

/// 解析有限参数；`--help` 返回 None，不接受秘密值或隐式默认根。
pub fn parse(args: Vec<OsString>) -> Result<Option<(Command, Options)>, Error> {
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    if args.len() < 13 || args.len() > 141 || args.len() % 2 != 1 {
        return Err(Error::Arguments);
    }
    let name = args[0].to_str().ok_or(Error::Arguments)?;
    if !matches!(name, "plan" | "initialize") {
        return Err(Error::Arguments);
    }
    let mut fields = BTreeMap::new();
    let mut allow = Vec::new();
    for pair in args[1..].as_chunks::<2>().0 {
        let key = pair[0].to_str().ok_or(Error::Arguments)?;
        let value = pair[1].to_str().ok_or(Error::Arguments)?;
        if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(Error::Arguments);
        }
        if key == "--allow" {
            allow.push(VaultPath::parse(value).map_err(|_| Error::Arguments)?);
        } else if !matches!(
            key,
            "--config"
                | "--vault"
                | "--writer-data"
                | "--assets"
                | "--source-snapshot"
                | "--expected-plan"
        ) || fields.insert(key, value).is_some()
        {
            return Err(Error::Arguments);
        }
    }
    let command = match (name, fields.remove("--expected-plan")) {
        ("plan", None) => Command::Plan,
        ("initialize", Some(expected)) => {
            super::decode_digest(expected)?;
            Command::Initialize {
                expected_plan: expected.into(),
            }
        }
        _ => return Err(Error::Arguments),
    };
    let mut path = |key| {
        fields
            .remove(key)
            .map(PathBuf::from)
            .ok_or(Error::Arguments)
    };
    let options = Options {
        config: path("--config")?,
        vault: path("--vault")?,
        writer_data: path("--writer-data")?,
        assets: path("--assets")?,
        source_snapshot: path("--source-snapshot")?,
        allow,
    };
    if options.allow.is_empty() || options.allow.len() > 64 {
        return Err(Error::Arguments);
    }
    Ok(Some((command, options)))
}
