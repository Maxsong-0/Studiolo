//! 封闭的命令行参数解析：只接受已知参数，不读环境变量或配置文件。

use std::path::PathBuf;

/// 用法说明。
pub const USAGE: &str = "studiolo-agent：Studiolo 供给面 MCP 主外壳（M3）\n\n\
用法：\n  \
studiolo-agent serve --vault </绝对路径/vault> --allow <相对目录> [--allow …] --data </绝对路径/私有目录> [--port 4400]\n  \
studiolo-agent admin init --data </绝对路径/私有目录>\n  \
studiolo-agent admin type-issue --data <私有目录> --type <claude|codex|grok|…>\n  \
studiolo-agent admin type-revoke --data <私有目录> --type <类型名>\n  \
studiolo-agent admin types --data <私有目录>\n  \
studiolo-agent admin reads --data <私有目录> [--type T] [--instance I] [--since RFC3339] [--until RFC3339] [--path-prefix P] [--limit N]\n  \
studiolo-agent admin rebuild-read-index --data <私有目录>\n  \
studiolo-agent admin outages --data <私有目录>\n\n\
私有目录及其 audit/ 子目录须预先以 0700 创建；服务只监听 127.0.0.1，客户端须带 Authorization: Bearer <类型 token>。";

/// `serve` 参数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeOptions {
    /// vault 根（绝对路径）。
    pub vault: PathBuf,
    /// 白名单目录（相对 vault 根）。
    pub allow: Vec<String>,
    /// 私有数据目录（绝对路径）。
    pub data: PathBuf,
    /// loopback 端口。
    pub port: u16,
}

/// `admin` 子命令。
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)] // 字段含义见 USAGE；每个变体只有私有目录与显式过滤项
pub enum AdminCommand {
    /// 初始化私有库与审计账本。
    Init { data: PathBuf },
    /// 签发或轮换类型 token。
    TypeIssue { data: PathBuf, agent_type: String },
    /// 吊销类型。
    TypeRevoke { data: PathBuf, agent_type: String },
    /// 列出类型。
    Types { data: PathBuf },
    /// 四维读取审计查询。
    Reads {
        data: PathBuf,
        agent_type: Option<String>,
        instance: Option<String>,
        since: Option<String>,
        until: Option<String>,
        path_prefix: Option<String>,
        limit: usize,
    },
    /// 从账本重建读取索引。
    RebuildReadIndex { data: PathBuf },
    /// 列出审计不可用窗口。
    Outages { data: PathBuf },
}

/// 顶层命令。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// 打印用法。
    Help,
    /// 启动服务。
    Serve(ServeOptions),
    /// 管理子命令。
    Admin(AdminCommand),
}

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
                if !matches!(key.as_str(), "--vault" | "--allow" | "--data" | "--port") {
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
            Ok(Command::Serve(ServeOptions {
                vault: absolute(required(&pairs, "--vault")?, "--vault")?,
                allow,
                data: absolute(required(&pairs, "--data")?, "--data")?,
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
                "outages" => AdminCommand::Outages { data: data()? },
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
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn serve_requires_absolute_paths_and_at_least_one_allow() {
        let parsed = parse(&strings(&[
            "serve",
            "--vault",
            "/v",
            "--allow",
            "项目/Studiolo",
            "--allow",
            "_规则",
            "--data",
            "/d",
        ]))
        .unwrap();
        assert_eq!(
            parsed,
            Command::Serve(ServeOptions {
                vault: PathBuf::from("/v"),
                allow: vec!["项目/Studiolo".into(), "_规则".into()],
                data: PathBuf::from("/d"),
                port: 4400,
            })
        );
        assert!(
            parse(&strings(&[
                "serve", "--vault", "v", "--allow", "a", "--data", "/d"
            ]))
            .is_err()
        );
        assert!(parse(&strings(&["serve", "--vault", "/v", "--data", "/d"])).is_err());
        assert!(
            parse(&strings(&[
                "serve", "--vault", "/v", "--allow", "a", "--data", "/d", "--port", "80"
            ]))
            .is_err()
        );
        assert!(
            parse(&strings(&[
                "serve", "--vault", "/v", "--allow", "a", "--data", "/d", "--extra", "x"
            ]))
            .is_err()
        );
    }

    #[test]
    fn admin_subcommands_are_closed() {
        assert_eq!(
            parse(&strings(&[
                "admin",
                "type-issue",
                "--data",
                "/d",
                "--type",
                "claude"
            ]))
            .unwrap(),
            Command::Admin(AdminCommand::TypeIssue {
                data: PathBuf::from("/d"),
                agent_type: "claude".into()
            })
        );
        assert!(parse(&strings(&["admin", "type-issue", "--data", "/d"])).is_err());
        assert!(parse(&strings(&["admin", "init", "--data", "/d", "--type", "x"])).is_err());
        assert!(parse(&strings(&["admin", "nuke", "--data", "/d"])).is_err());
        assert!(matches!(parse(&strings(&[])).unwrap(), Command::Help));
        assert!(matches!(
            parse(&strings(&[
                "admin", "reads", "--data", "/d", "--limit", "5"
            ]))
            .unwrap(),
            Command::Admin(AdminCommand::Reads { limit: 5, .. })
        ));
    }
}
