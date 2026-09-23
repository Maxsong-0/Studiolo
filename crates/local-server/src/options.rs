//! 纯参数解析 Interface：不读环境、不连接网络、不打开副本。

mod table;

use std::{collections::HashSet, path::PathBuf};
use studiolo_local_server::AccessConfig;
use table::{Arity, PARAMS};

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
    pub(crate) project_repository: Option<(String, PathBuf)>,
    pub(crate) cargo_dependencies: bool,
    pub(crate) rust_call_sites: Option<Vec<String>>,
    pub(crate) policy_data: Option<PathBuf>,
    pub(crate) board: Option<BoardOptions>,
    pub(crate) runner_config: Option<PathBuf>,
}

pub(crate) struct BoardOptions {
    pub(crate) socket: PathBuf,
    pub(crate) data: PathBuf,
    pub(crate) agent_data: PathBuf,
}

pub(crate) struct SessionOptions {
    pub(crate) config: PathBuf,
    pub(crate) origin: String,
}

pub(crate) struct FactorOptions {
    pub(crate) socket: PathBuf,
    pub(crate) replacement_seed_ref: Option<String>,
}

/// 逐个实参累积的原始取值：由参数表的取值处理写入，跨参数约束等全部实参读完后统一核验。
#[derive(Default)]
struct Raw {
    help: bool,
    canonical: bool,
    cargo_dependencies: bool,
    vault: Option<PathBuf>,
    assets: Option<PathBuf>,
    label: Option<String>,
    port: Option<u16>,
    public_origin: Option<String>,
    team: Option<String>,
    audience: Option<String>,
    email: Option<String>,
    session_config: Option<PathBuf>,
    writer_data: Option<PathBuf>,
    factor_socket: Option<PathBuf>,
    replacement_seed_ref: Option<String>,
    project_repository: Option<(String, PathBuf)>,
    rust_call_sites: Option<Vec<String>>,
    policy_data: Option<PathBuf>,
    runner_config: Option<PathBuf>,
    board_socket: Option<PathBuf>,
    board_data: Option<PathBuf>,
    board_agent_data: Option<PathBuf>,
}

pub(crate) fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut seen = HashSet::new();
    let mut raw = Raw::default();
    while let Some(argument) = args.next() {
        let key = if argument == "-h" {
            "--help"
        } else {
            argument.as_str()
        };
        let Some((name, arity)) = PARAMS.iter().find(|(name, _)| *name == key) else {
            return Err("未知参数；仅接受 --help 中的非秘密参数".into());
        };
        if !seen.insert(*name) {
            return Err(format!("参数 {key} 不得重复"));
        }
        let apply = match arity {
            Arity::Flag(apply) => {
                apply(&mut raw);
                continue;
            }
            Arity::Value(apply) => apply,
        };
        let value = args
            .next()
            .filter(|value| !value.is_empty() && !value.starts_with("--") && value != "-h")
            .ok_or_else(|| format!("参数 {key} 缺少值"))?;
        apply(&mut raw, value)?;
    }
    let (access, session) = if let Some(config) = raw.session_config {
        if raw.team.is_some() || raw.audience.is_some() || raw.email.is_some() {
            return Err(
                "会话模式不得混用 --access-team、--access-audience 或 --access-email".into(),
            );
        }
        let origin = raw
            .public_origin
            .ok_or("会话模式必须显式提供 --public-origin")?;
        if !config.is_absolute() {
            return Err("会话配置必须使用绝对路径".into());
        }
        (None, Some(SessionOptions { config, origin }))
    } else {
        let access = match (raw.public_origin, raw.team, raw.audience, raw.email) {
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
    let factor = if let Some(socket) = raw.factor_socket {
        if session.is_none() || seen.contains("--port") || !socket.is_absolute() {
            return Err("因子模式需要会话配置、公共Origin和绝对socket路径，不得指定TCP端口".into());
        }
        Some(FactorOptions {
            socket,
            replacement_seed_ref: raw.replacement_seed_ref,
        })
    } else {
        if raw.replacement_seed_ref.is_some() {
            return Err("候选因子引用仅能用于显式 --factor-socket 模式".into());
        }
        None
    };
    if raw.help {
        return Ok(None);
    }
    if raw.project_repository.is_some() && session.is_some() {
        return Err(
            "仓库目录绑定首片仅用于本机或旧Access只读模式，不能混用会话/canonical/因子模式".into(),
        );
    }
    if raw.cargo_dependencies && raw.project_repository.is_none() {
        return Err("Cargo清单读取必须配合显式 --project-repository".into());
    }
    if raw.rust_call_sites.is_some() && raw.project_repository.is_none() {
        return Err("Rust源码读取必须配合显式 --project-repository".into());
    }
    if raw.canonical && session.is_none() {
        return Err("canonical读取必须配合显式 --session-config 及公共Origin".into());
    }
    if raw
        .policy_data
        .as_ref()
        .is_some_and(|path| !path.is_absolute() || session.is_none())
    {
        return Err("策略平面私有目录必须为绝对路径并配合会话配置".into());
    }
    if raw
        .writer_data
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
        || (raw.writer_data.is_some() && (raw.canonical || session.is_none()))
    {
        return Err("canonical写者数据必须为绝对路径并配合会话配置，不得混用 --canonical".into());
    }
    let board = match (raw.board_socket, raw.board_data, raw.board_agent_data) {
        (None, None, None) => None,
        (Some(socket), Some(data), Some(agent_data))
            if raw.writer_data.is_some()
                && raw.policy_data.is_some()
                && socket.is_absolute()
                && data.is_absolute()
                && agent_data.is_absolute() =>
        {
            Some(BoardOptions {
                socket,
                data,
                agent_data,
            })
        }
        _ => return Err("黑板必须同时指定三个绝对路径、canonical写者及策略目录".into()),
    };
    if raw
        .runner_config
        .as_ref()
        .is_some_and(|p| !p.is_absolute() || board.is_none() || factor.is_none())
    {
        return Err("执行体需规范绝对配置路径、黑板和可信二次认证入口".into());
    }
    Ok(Some(Options {
        vault: raw
            .vault
            .ok_or("必须显式提供 --vault <副本目录>，不会默认访问真实笔记库")?,
        assets: raw.assets.unwrap_or_else(|| PathBuf::from("web/dist")),
        label: raw.label.unwrap_or_else(|| "只读副本".to_owned()),
        port: raw.port.unwrap_or(4317),
        access,
        session,
        canonical: raw.canonical,
        writer_data: raw.writer_data,
        factor,
        project_repository: raw.project_repository,
        cargo_dependencies: raw.cargo_dependencies,
        rust_call_sites: raw.rust_call_sites,
        policy_data: raw.policy_data,
        board,
        runner_config: raw.runner_config,
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

#[cfg(test)]
#[path = "../tests/options/repository.rs"]
mod repository_tests;

#[cfg(test)]
#[path = "../tests/options/cargo_dependencies.rs"]
mod cargo_dependency_tests;

#[cfg(test)]
#[path = "../tests/options/rust_call_sites.rs"]
mod rust_call_tests;

#[cfg(test)]
#[path = "../tests/options/table.rs"]
mod table_tests;
