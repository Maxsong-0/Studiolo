//! 封闭的命令行参数解析：只接受已知参数，不读环境变量或配置文件。

use std::path::PathBuf;

/// 用法说明。
pub const USAGE: &str = "studiolo-agent：Studiolo 供给面 MCP 主外壳（M3）\n\n\
用法：\n  \
studiolo-agent serve (--vault </绝对路径/vault> | --replica-root </绝对路径/副本>) [--replica-max-age-secs 300] --allow <相对目录> [--allow …] --data </绝对路径/私有目录> [--policy </绝对路径/策略私有目录>] [--board-socket </绝对路径/黑板socket>] [--index-cache </独立700目录>] [--port 4400]\n  \
studiolo-agent admin identity-upgrade --data <私有目录>（启用子实例；离线显式升级）\n  \
studiolo-agent admin init --data </绝对路径/私有目录>\n  \
studiolo-agent admin type-issue --data <私有目录> --type <claude|codex|grok|…>\n  \
studiolo-agent admin type-revoke --data <私有目录> --type <类型名>\n  \
studiolo-agent admin types --data <私有目录>\n  \
studiolo-agent admin reads --data <私有目录> [--type T] [--instance I] [--since RFC3339] [--until RFC3339] [--path-prefix P] [--limit N]\n  \
studiolo-agent admin rebuild-read-index --data <私有目录>\n  \
studiolo-agent admin audit-upgrade --data <私有目录>   # T-012 v1 账本 → 分段格式，一次性、单向\n  \
studiolo-agent admin outages --data <私有目录>\n  \
studiolo-agent admin instances --data <私有目录> [--limit N]\n  \
studiolo-agent admin policy-init --policy <策略私有目录>\n  \
studiolo-agent admin policy-bootstrap --policy <策略私有目录> --data <私有目录> --kind <rules|skills|capability_set> --subject <名> --file </绝对路径/正文>（只能写首版；须在真实前台控制终端运行）\n  \
studiolo-agent admin policy-status --policy <策略私有目录>\n\n\
私有目录及其 audit/ 子目录须预先以 0700 创建；服务只监听 127.0.0.1，客户端须带 Authorization: Bearer <类型 token>。";

/// `serve` 参数。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServeOptions {
    /// vault 根（绝对路径）。
    pub vault: Option<PathBuf>,
    /// 不可变副本根；与 vault 二选一。
    pub replica_root: Option<PathBuf>,
    /// 副本新鲜度阈值，60..=86400 秒。
    pub replica_max_age_secs: u64,
    /// 白名单目录（相对 vault 根）。
    pub allow: Vec<String>,
    /// 私有数据目录（绝对路径）。
    pub data: PathBuf,
    /// 策略平面私有目录（绝对路径）；缺省为 M3 兼容模式（无能力集、无吊销表）。
    pub policy: Option<PathBuf>,
    /// canonical 单写者的私有黑板 Unix 入口；必须同时配置策略。
    pub board_socket: Option<PathBuf>,
    /// 可丢弃的派生索引缓存；显式独立私有目录，不能与源或身份目录重叠。
    pub index_cache: Option<PathBuf>,
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
    /// 把 T-012 的 v1 审计账本显式升级为分段格式（一次性、单向；v2 或校验不过一律拒绝）。
    AuditUpgrade { data: PathBuf },
    /// 显式升级子实例身份关系。
    IdentityUpgrade { data: PathBuf },
    /// 列出审计不可用窗口。
    Outages { data: PathBuf },
    /// 列出已注册实例。
    Instances { data: PathBuf, limit: usize },
    /// 初始化策略平面私有库。
    PolicyInit { policy: PathBuf },
    /// 首版 bootstrap（真实控制终端门禁；只允许从未有过版本的 subject）。
    PolicyBootstrap {
        policy: PathBuf,
        data: PathBuf,
        kind: String,
        subject: String,
        file: PathBuf,
    },
    /// 打印 effective 摘要、现行各项与生效中的吊销。
    PolicyStatus { policy: PathBuf },
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
mod tests;
