//! 纯参数解析 Interface：不读环境、不连接网络、不打开副本。

use std::{collections::HashSet, path::PathBuf};
use studiolo_local_server::AccessConfig;

pub(crate) struct Options {
    pub(crate) vault: PathBuf,
    pub(crate) assets: PathBuf,
    pub(crate) label: String,
    pub(crate) port: u16,
    pub(crate) access: Option<AccessConfig>,
    pub(crate) session: Option<SessionOptions>,
    pub(crate) canonical: bool,
    pub(crate) writer_data: Option<PathBuf>,
    pub(crate) factor: Option<FactorOptions>,
}

pub(crate) struct SessionOptions {
    pub(crate) config: PathBuf,
    pub(crate) origin: String,
}

pub(crate) struct FactorOptions {
    pub(crate) socket: PathBuf,
    pub(crate) replacement_seed_ref: Option<String>,
}

pub(crate) fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut seen = HashSet::new();
    let mut help = false;
    let mut canonical = false;
    let mut writer_data = None;
    let mut vault = None;
    let mut session_config = None;
    let mut factor_socket = None;
    let mut replacement_seed_ref = None;
    let mut assets = PathBuf::from("web/dist");
    let mut label = "只读副本".to_owned();
    let mut port = 4317;
    let (mut public_origin, mut team, mut audience, mut email) = (None, None, None, None);
    while let Some(argument) = args.next() {
        let key = if argument == "-h" {
            "--help"
        } else {
            argument.as_str()
        };
        if !matches!(
            key,
            "--vault"
                | "--assets"
                | "--label"
                | "--port"
                | "--public-origin"
                | "--access-team"
                | "--access-audience"
                | "--access-email"
                | "--session-config"
                | "--canonical"
                | "--canonical-writer-data"
                | "--factor-socket"
                | "--replacement-seed-ref"
                | "--help"
        ) {
            return Err("未知参数；仅接受 --help 中的非秘密参数".into());
        }
        if !seen.insert(key.to_owned()) {
            return Err(format!("参数 {key} 不得重复"));
        }
        if key == "--help" {
            help = true;
            continue;
        }
        if key == "--canonical" {
            canonical = true;
            continue;
        }
        let value = args
            .next()
            .filter(|value| !value.is_empty() && !value.starts_with("--") && value != "-h")
            .ok_or_else(|| format!("参数 {key} 缺少值"))?;
        match key {
            "--vault" => vault = Some(PathBuf::from(value)),
            "--canonical-writer-data" => writer_data = Some(PathBuf::from(value)),
            "--session-config" => session_config = Some(PathBuf::from(value)),
            "--factor-socket" => factor_socket = Some(PathBuf::from(value)),
            "--replacement-seed-ref" => replacement_seed_ref = Some(value),
            "--assets" => assets = PathBuf::from(value),
            "--label" => label = value,
            "--port" => {
                port = value
                    .parse::<u16>()
                    .ok()
                    .filter(|p| *p != 0)
                    .ok_or("端口必须为 1–65535")?;
            }
            "--public-origin" => public_origin = Some(value),
            "--access-team" => team = Some(value),
            "--access-audience" => audience = Some(value),
            "--access-email" => email = Some(value),
            _ => unreachable!("仅已知的带值参数到达此处"),
        }
    }
    let (access, session) = if let Some(config) = session_config {
        if team.is_some() || audience.is_some() || email.is_some() {
            return Err(
                "会话模式不得混用 --access-team、--access-audience 或 --access-email".into(),
            );
        }
        let origin = public_origin.ok_or("会话模式必须显式提供 --public-origin")?;
        if !config.is_absolute() {
            return Err("会话配置必须使用绝对路径".into());
        }
        (None, Some(SessionOptions { config, origin }))
    } else {
        let access = match (public_origin, team, audience, email) {
            (None, None, None, None) => None,
            (Some(origin), Some(team), Some(audience), Some(email)) => Some(
                AccessConfig::new(origin, team, audience, email)
                    .map_err(|error| format!("公共入口配置无效：{error}"))?,
            ),
            _ => {
                return Err(
                "公共入口必须同时提供 --public-origin、--access-team、--access-audience、--access-email"
                    .into(),
            );
            }
        };
        (access, None)
    };
    let factor = if let Some(socket) = factor_socket {
        if session.is_none() || seen.contains("--port") || !socket.is_absolute() {
            return Err("因子模式需要会话配置、公共Origin和绝对socket路径，不得指定TCP端口".into());
        }
        Some(FactorOptions {
            socket,
            replacement_seed_ref,
        })
    } else {
        if replacement_seed_ref.is_some() {
            return Err("候选因子引用仅能用于显式 --factor-socket 模式".into());
        }
        None
    };
    if help {
        return Ok(None);
    }
    if canonical && session.is_none() {
        return Err("canonical读取必须配合显式 --session-config 及公共Origin".into());
    }
    if writer_data.as_ref().is_some_and(|path| !path.is_absolute())
        || (writer_data.is_some() && (canonical || session.is_none()))
    {
        return Err("canonical写者数据必须为绝对路径并配合会话配置，不得混用 --canonical".into());
    }
    Ok(Some(Options {
        vault: vault.ok_or("必须显式提供 --vault <副本目录>，不会默认访问真实笔记库")?,
        assets,
        label,
        port,
        access,
        session,
        canonical,
        writer_data,
        factor,
    }))
}

#[cfg(test)]
#[path = "../tests/options/legacy.rs"]
mod legacy_tests;

#[cfg(test)]
#[path = "../tests/options/sessions.rs"]
mod session_tests;

#[cfg(test)]
#[path = "../tests/options/writer.rs"]
mod writer_tests;
