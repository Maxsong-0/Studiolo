//! 启动参数表：可接受参数的唯一来源（名字 → 读法与取值处理），各值在读到时就地解析。

use std::path::PathBuf;

use super::Raw;

/// 参数读法：开关只置位；带值参数先取紧随的实参，再交给取值处理，其错误即对外文案。
pub(super) enum Arity {
    Flag(fn(&mut Raw)),
    Value(fn(&mut Raw, String) -> Result<(), String>),
}

/// 表外名字一律按未知参数拒绝；新增参数只改这一处，解析循环无需另记名单。
pub(super) const PARAMS: &[(&str, Arity)] = &[
    (
        "--vault",
        Arity::Value(|raw, value| set(&mut raw.vault, value.into())),
    ),
    (
        "--assets",
        Arity::Value(|raw, value| set(&mut raw.assets, value.into())),
    ),
    (
        "--label",
        Arity::Value(|raw, value| set(&mut raw.label, value)),
    ),
    (
        "--port",
        Arity::Value(|raw, value| set(&mut raw.port, parse_port(&value)?)),
    ),
    (
        "--public-origin",
        Arity::Value(|raw, value| set(&mut raw.public_origin, value)),
    ),
    (
        "--access-team",
        Arity::Value(|raw, value| set(&mut raw.team, value)),
    ),
    (
        "--access-audience",
        Arity::Value(|raw, value| set(&mut raw.audience, value)),
    ),
    (
        "--access-email",
        Arity::Value(|raw, value| set(&mut raw.email, value)),
    ),
    (
        "--session-config",
        Arity::Value(|raw, value| set(&mut raw.session_config, value.into())),
    ),
    ("--canonical", Arity::Flag(|raw| raw.canonical = true)),
    (
        "--canonical-writer-data",
        Arity::Value(|raw, value| set(&mut raw.writer_data, value.into())),
    ),
    (
        "--factor-socket",
        Arity::Value(|raw, value| set(&mut raw.factor_socket, value.into())),
    ),
    (
        "--replacement-seed-ref",
        Arity::Value(|raw, value| set(&mut raw.replacement_seed_ref, value)),
    ),
    (
        "--project-repository",
        Arity::Value(|raw, value| set(&mut raw.project_repository, parse_repository(&value)?)),
    ),
    (
        "--cargo-dependencies",
        Arity::Flag(|raw| raw.cargo_dependencies = true),
    ),
    (
        "--rust-call-sites",
        Arity::Value(|raw, value| set(&mut raw.rust_call_sites, parse_rust_sources(&value)?)),
    ),
    (
        "--policy-data",
        Arity::Value(|raw, value| set(&mut raw.policy_data, value.into())),
    ),
    (
        "--runner-config",
        Arity::Value(|raw, value| set(&mut raw.runner_config, value.into())),
    ),
    (
        "--board-socket",
        Arity::Value(|raw, value| set(&mut raw.board_socket, value.into())),
    ),
    (
        "--board-data",
        Arity::Value(|raw, value| set(&mut raw.board_data, value.into())),
    ),
    (
        "--board-agent-data",
        Arity::Value(|raw, value| set(&mut raw.board_agent_data, value.into())),
    ),
    ("--help", Arity::Flag(|raw| raw.help = true)),
];

/// 取值处理的公共落点：重复参数在查表后已被拒绝，这里只写入对应槽位。
fn set<T>(slot: &mut Option<T>, value: T) -> Result<(), String> {
    *slot = Some(value);
    Ok(())
}

fn parse_port(value: &str) -> Result<u16, String> {
    let port = value.parse::<u16>().ok().filter(|port| *port != 0);
    port.ok_or_else(|| "端口必须为 1–65535".to_owned())
}

fn parse_rust_sources(value: &str) -> Result<Vec<String>, String> {
    if value.len() > 32 * 1024 {
        return Err("Rust源码白名单参数超过32KiB预算".into());
    }
    let paths: Vec<String> = serde_json::from_str(value)
        .map_err(|_| "Rust源码白名单必须是JSON字符串数组；不回显输入".to_owned())?;
    studiolo_workbench::validate_rust_source_paths(&paths).map_err(|error| error.message)
}

fn parse_repository(value: &str) -> Result<(String, PathBuf), String> {
    let invalid = || "仓库绑定须为 项目/名称=/绝对仓库路径；不回显输入".to_owned();
    let (project, root) = value.split_once('=').ok_or_else(invalid)?;
    let path = studiolo_core::VaultPath::parse(project).map_err(|_| invalid())?;
    let parts: Vec<_> = path.as_str().split('/').collect();
    if parts.len() != 2
        || parts[0] != "项目"
        || parts[1].starts_with('.')
        || !PathBuf::from(root).is_absolute()
        || root.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    Ok((project.to_owned(), PathBuf::from(root)))
}
