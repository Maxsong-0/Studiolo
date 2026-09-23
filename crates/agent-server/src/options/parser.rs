//! 命令行校验：切分键值对，逐项校验允许的参数、出现次数、绝对路径与取值范围，再组装命令。

use std::path::PathBuf;

use super::{AdminCommand, Command, ServeOptions};

fn take_values(args: &[String]) -> Result<Vec<(String, Option<String>)>, String> {
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let key = &args[index];
        if !key.starts_with("--") {
            return Err(format!("未知参数 {key}"));
        }
        let value = args.get(index + 1).filter(|value| !value.starts_with("--"));
        if let Some(value) = value {
            pairs.push((key.clone(), Some(value.clone())));
            index += 2;
        } else {
            pairs.push((key.clone(), None));
            index += 1;
        }
    }
    Ok(pairs)
}

fn absolute(value: &str, what: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || value.is_empty() {
        return Err(format!("{what} 必须是绝对路径"));
    }
    Ok(path)
}

fn required<'a>(pairs: &'a [(String, Option<String>)], key: &str) -> Result<&'a str, String> {
    let found = pairs
        .iter()
        .filter(|(name, _)| name == key)
        .collect::<Vec<_>>();
    match found.as_slice() {
        [(_, Some(value))] => Ok(value.as_str()),
        [] => Err(format!("缺少 {key}")),
        _ => Err(format!("{key} 只能出现一次且必须有值")),
    }
}

fn optional<'a>(
    pairs: &'a [(String, Option<String>)],
    key: &str,
) -> Result<Option<&'a str>, String> {
    let found = pairs
        .iter()
        .filter(|(name, _)| name == key)
        .collect::<Vec<_>>();
    match found.as_slice() {
        [] => Ok(None),
        [(_, Some(value))] => Ok(Some(value.as_str())),
        _ => Err(format!("{key} 只能出现一次且必须有值")),
    }
}

/// 解析参数；`--help` 或无参数返回 `Help`。
pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Ok(Command::Help);
    };
    match first.as_str() {
        "--help" | "-h" | "help" => Ok(Command::Help),
        "serve" => {
            let pairs = take_values(&args[1..])?;
            for (key, _) in &pairs {
                if !matches!(
                    key.as_str(),
                    "--vault"
                        | "--replica-root"
                        | "--replica-max-age-secs"
                        | "--allow"
                        | "--data"
                        | "--policy"
                        | "--board-socket"
                        | "--port"
                        | "--index-cache"
                ) {
                    return Err(format!("serve 不接受参数 {key}"));
                }
            }
            let allow = pairs
                .iter()
                .filter(|(name, _)| name == "--allow")
                .map(|(_, value)| value.clone().ok_or_else(|| "--allow 必须有值".to_owned()))
                .collect::<Result<Vec<_>, _>>()?;
            if allow.is_empty() {
                return Err("至少一个 --allow".into());
            }
            let port = match optional(&pairs, "--port")? {
                None => 4400,
                Some(value) => value
                    .parse::<u16>()
                    .ok()
                    .filter(|port| *port >= 1024)
                    .ok_or_else(|| "--port 必须是 1024..=65535 的整数".to_owned())?,
            };
            let vault = optional(&pairs, "--vault")?
                .map(|value| absolute(value, "--vault"))
                .transpose()?;
            let replica_root = optional(&pairs, "--replica-root")?
                .map(|value| absolute(value, "--replica-root"))
                .transpose()?;
            if vault.is_some() == replica_root.is_some() {
                return Err("--vault 与 --replica-root 必须且只能指定一个".into());
            }
            let max_age = optional(&pairs, "--replica-max-age-secs")?;
            if max_age.is_some() && replica_root.is_none() {
                return Err("--replica-max-age-secs 只适用于 --replica-root".into());
            }
            let replica_max_age_secs = max_age.map_or(Ok(300), |value| {
                value
                    .parse::<u64>()
                    .ok()
                    .filter(|age| (60..=86_400).contains(age))
                    .ok_or_else(|| "--replica-max-age-secs 必须是 60..=86400".to_owned())
            })?;
            Ok(Command::Serve(ServeOptions {
                vault,
                replica_root,
                replica_max_age_secs,
                allow,
                data: absolute(required(&pairs, "--data")?, "--data")?,
                policy: optional(&pairs, "--policy")?
                    .map(|value| absolute(value, "--policy"))
                    .transpose()?,
                board_socket: optional(&pairs, "--board-socket")?
                    .map(|value| absolute(value, "--board-socket"))
                    .transpose()?,
                index_cache: optional(&pairs, "--index-cache")?
                    .map(|value| absolute(value, "--index-cache"))
                    .transpose()?,
                port,
            }))
        }
        "admin" => {
            let Some(sub) = args.get(1) else {
                return Err("admin 缺少子命令".into());
            };
            let pairs = take_values(&args[2..])?;
            let data = || absolute(required(&pairs, "--data")?, "--data");
            let command = match sub.as_str() {
                "init" => AdminCommand::Init { data: data()? },
                "type-issue" => AdminCommand::TypeIssue {
                    data: data()?,
                    agent_type: required(&pairs, "--type")?.to_owned(),
                },
                "type-revoke" => AdminCommand::TypeRevoke {
                    data: data()?,
                    agent_type: required(&pairs, "--type")?.to_owned(),
                },
                "types" => AdminCommand::Types { data: data()? },
                "reads" => AdminCommand::Reads {
                    data: data()?,
                    agent_type: optional(&pairs, "--type")?.map(str::to_owned),
                    instance: optional(&pairs, "--instance")?.map(str::to_owned),
                    since: optional(&pairs, "--since")?.map(str::to_owned),
                    until: optional(&pairs, "--until")?.map(str::to_owned),
                    path_prefix: optional(&pairs, "--path-prefix")?.map(str::to_owned),
                    limit: match optional(&pairs, "--limit")? {
                        None => 200,
                        Some(value) => value
                            .parse::<usize>()
                            .ok()
                            .filter(|limit| (1..=10_000).contains(limit))
                            .ok_or_else(|| "--limit 必须是 1..=10000".to_owned())?,
                    },
                },
                "rebuild-read-index" => AdminCommand::RebuildReadIndex { data: data()? },
                "audit-upgrade" => AdminCommand::AuditUpgrade { data: data()? },
                "identity-upgrade" => AdminCommand::IdentityUpgrade { data: data()? },
                "outages" => AdminCommand::Outages { data: data()? },
                "instances" => AdminCommand::Instances {
                    data: data()?,
                    limit: match optional(&pairs, "--limit")? {
                        None => 200,
                        Some(value) => value
                            .parse::<usize>()
                            .ok()
                            .filter(|limit| (1..=10_000).contains(limit))
                            .ok_or_else(|| "--limit 必须是 1..=10000".to_owned())?,
                    },
                },
                "policy-init" => AdminCommand::PolicyInit {
                    policy: absolute(required(&pairs, "--policy")?, "--policy")?,
                },
                "policy-bootstrap" => AdminCommand::PolicyBootstrap {
                    policy: absolute(required(&pairs, "--policy")?, "--policy")?,
                    data: data()?,
                    kind: required(&pairs, "--kind")?.to_owned(),
                    subject: required(&pairs, "--subject")?.to_owned(),
                    file: absolute(required(&pairs, "--file")?, "--file")?,
                },
                "policy-status" => AdminCommand::PolicyStatus {
                    policy: absolute(required(&pairs, "--policy")?, "--policy")?,
                },
                other => return Err(format!("未知 admin 子命令 {other}")),
            };
            let allowed: &[&str] = match sub.as_str() {
                "reads" => &[
                    "--data",
                    "--type",
                    "--instance",
                    "--since",
                    "--until",
                    "--path-prefix",
                    "--limit",
                ],
                "type-issue" | "type-revoke" => &["--data", "--type"],
                "instances" => &["--data", "--limit"],
                "policy-init" | "policy-status" => &["--policy"],
                "policy-bootstrap" => &["--policy", "--data", "--kind", "--subject", "--file"],
                _ => &["--data"],
            };
            for (key, _) in &pairs {
                if !allowed.contains(&key.as_str()) {
                    return Err(format!("{sub} 不接受参数 {key}"));
                }
            }
            Ok(Command::Admin(command))
        }
        other => Err(format!("未知命令 {other}")),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
