//! 可信本地管理子命令：初始化私有库与账本、签发/吊销类型 token、读取审计查询、索引重建、outage 列表。
//!
//! 秘密只有一条出口：`type-issue` 的类型 token 只写真实前台控制终端（见 `terminal`），
//! 标准输出永远只承载非秘密的固定结果，因此重定向、管道与日志采集都拿不到明文。

mod policy;
mod terminal;

use std::{io::Write as _, path::Path};

use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::{AgentStore, ReadQuery, ReadRecord, audit::CONTENT_READ};

use crate::{options::AdminCommand, state::now};

fn parse_time(value: &str) -> Result<i64, String> {
    value
        .parse::<jiff::Timestamp>()
        .map(|timestamp| timestamp.as_millisecond())
        .map_err(|_| format!("时间必须是 RFC 3339：{value}"))
}

/// 执行一个管理子命令；唯一的秘密（类型 token）只在真实前台控制终端显示一次。
///
/// 其余子命令只输出非秘密结果，允许重定向与管道，便于运维把查询结果落盘。
pub fn run(command: AdminCommand) -> Result<(), String> {
    match command {
        AdminCommand::Init { data } => {
            AgentStore::open(&data).map_err(|error| format!("初始化 agent 私有库失败：{error}"))?;
            SqliteAuditLedger::open(&data.join("audit")).map_err(|_| {
                "初始化审计账本失败：需要预先存在的 0700 目录 <data>/audit".to_owned()
            })?;
            println!("已初始化：agent.sqlite3 与 audit/audit.sqlite3");
            Ok(())
        }
        AdminCommand::TypeIssue { data, agent_type } => issue_type(&data, &agent_type),
        AdminCommand::TypeRevoke { data, agent_type } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            let revoked = store
                .revoke_type(&agent_type, &now())
                .map_err(|error| error.to_string())?;
            println!(
                "{}",
                if revoked {
                    "已吊销；新握手与后续调用即刻拒绝"
                } else {
                    "类型不存在或已吊销"
                }
            );
            Ok(())
        }
        AdminCommand::Types { data } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            for (name, issued_at, revoked) in store.types().map_err(|error| error.to_string())? {
                println!(
                    "{name}\t{issued_at}\t{}",
                    if revoked { "已吊销" } else { "有效" }
                );
            }
            Ok(())
        }
        AdminCommand::Reads {
            data,
            agent_type,
            instance,
            since,
            until,
            path_prefix,
            limit,
        } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            let query = ReadQuery {
                agent_type,
                instance_id: instance,
                since_ms: since.as_deref().map(parse_time).transpose()?,
                until_ms: until.as_deref().map(parse_time).transpose()?,
                path_prefix,
                limit,
            };
            for record in store
                .query_reads(&query)
                .map_err(|error| error.to_string())?
            {
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    record.recorded_at,
                    record.agent_type,
                    record.instance_id,
                    record.tool,
                    record.path,
                    &record.revision[..record.revision.len().min(12)],
                    record.ledger_offset
                );
            }
            Ok(())
        }
        AdminCommand::IdentityUpgrade { data } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            store
                .upgrade_child_instances()
                .map_err(|error| error.to_string())?;
            println!("身份库已显式升级；子实例权限逐次与祖先求交");
            Ok(())
        }
        AdminCommand::AuditUpgrade { data } => {
            // T-051 把账本磁盘格式升到 v2（分段摘要）。v1 账本 `open` 一律拒绝，必须先显式升级：
            // 升级先全链验证旧账本，再由已验证的记录链重算段摘要，不改写任何既有记录；
            // v2、未知版本或校验不过一律拒绝并回滚。成本与账本总量成正比，是人工运维动作。
            let directory = data.join("audit");
            let cost = SqliteAuditLedger::upgrade_existing(&directory)
                .map_err(|error| format!("升级审计账本失败（未改动任何记录）：{error}"))?;
            println!(
                "审计账本已升级为分段格式：全链验证 {} 条记录、{} 个段摘要",
                cost.records(),
                cost.summaries()
            );
            Ok(())
        }
        AdminCommand::RebuildReadIndex { data } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            let ledger = SqliteAuditLedger::open_existing(&data.join("audit"))
                .map_err(|_| "审计账本不可用".to_owned())?;
            let records = ledger
                .query(&AuditFilter::default().with_action(CONTENT_READ))
                .map_err(|error| error.to_string())?;
            let rebuilt = records.iter().filter_map(|record| {
                let payload = record.event().payload().as_value();
                Some(ReadRecord {
                    recorded_at_ms: record.recorded_at().as_millisecond(),
                    recorded_at: record.recorded_at().to_string(),
                    agent_type: payload.get("agent_type")?.as_str()?.to_owned(),
                    instance_id: payload.get("agent_instance")?.as_str()?.to_owned(),
                    path: payload.get("path")?.as_str()?.to_owned(),
                    revision: payload.get("revision")?.as_str()?.to_owned(),
                    tool: payload.get("tool")?.as_str()?.to_owned(),
                    ledger_offset: record.offset(),
                })
            });
            let count = store
                .rebuild_reads(rebuilt)
                .map_err(|error| error.to_string())?;
            println!("已从账本重建读取索引：{count} 条");
            Ok(())
        }
        AdminCommand::Instances { data, limit } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            for (instance, last_seen) in
                store.instances(limit).map_err(|error| error.to_string())?
            {
                println!(
                    "{}\t{}\t{}\t{last_seen}",
                    instance.id, instance.agent_type, instance.registered_at
                );
            }
            Ok(())
        }
        AdminCommand::PolicyInit { policy } => policy::init(&policy),
        AdminCommand::PolicyBootstrap {
            policy,
            data,
            kind,
            subject,
            file,
        } => policy::bootstrap(&policy, &data, &kind, &subject, &file),
        AdminCommand::PolicyStatus { policy } => policy::status(&policy),
        AdminCommand::Outages { data } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            for (id, started, ended, denied, reason) in
                store.outages().map_err(|error| error.to_string())?
            {
                println!(
                    "{id}\t{started}\t{}\t拒绝 {denied} 次\t{reason}",
                    ended.unwrap_or_else(|| "进行中".to_owned())
                );
            }
            Ok(())
        }
    }
}

/// 签发（或轮换）一个类型 token，并只在真实前台控制终端显示这一次。
///
/// 顺序是有意的：门禁 → 探写 → 签发 → 显示 → 非秘密确认。门禁与探写都排在签发之前，
/// 所以任何拒绝路径上都没有 token 被铸出，私有库状态也没有变；只有在通道已证明写得通
/// 之后才产生明文。显示失败时立刻吊销刚签发的凭据，绝不留下一个主人没拿到的活 token。
fn issue_type(data: &Path, agent_type: &str) -> Result<(), String> {
    let terminal = terminal::ControllingTerminal::open().map_err(refuse_before_issue)?;
    // 探写一行非秘密抬头：先证明这条通道真的写得通，再去铸 token。
    terminal
        .show("控制终端已核验：类型 token 只在此显示一次，不进标准输出、不落盘。\n")
        .map_err(refuse_before_issue)?;
    let store = AgentStore::open_existing(data).map_err(|error| error.to_string())?;
    let issued = store
        .issue_type(agent_type, &now())
        .map_err(|error| error.to_string())?;
    // 明文只经这一条通道；抬头与明文分开写，不把秘密再复制进一个格式化缓冲。
    if let Err(error) = terminal
        .show(&format!(
            "类型 {} 的 token（只显示这一次，库中只存 sha256）：\n",
            issued.agent_type
        ))
        .and_then(|()| terminal.show(&issued.token))
        .and_then(|()| terminal.show("\n"))
    {
        return Err(rollback(&store, agent_type, error));
    }
    writeln!(
        std::io::stdout(),
        "类型 {} 的 token 已在控制终端显示一次；本流不含秘密。请手抄进 0600 文件后清屏。",
        issued.agent_type
    )
    .map_err(|_| {
        "token 已签发并已在控制终端显示，但标准输出写入失败；库中状态已变，\
         请核对 `admin types --data <私有目录>` 后再决定是否轮换，不要自动重试。"
            .to_owned()
    })
}

/// 门禁在签发之前失败：明确拒绝并说明本次什么都没做，绝不降级到标准输出。
fn refuse_before_issue(error: terminal::TerminalError) -> String {
    format!(
        "拒绝签发类型 token：{error}。\n\
         本次未签发任何 token，私有库状态未变，也没有任何明文写到标准输出或标准错误。\n\
         请在交互终端直接运行 `studiolo-agent admin type-issue`：不要重定向、不要接管道、\
         不要写进脚本或日志采集；显示后由主人手工抄进 0600 文件。"
    )
}

/// 显示失败：token 已铸出而主人没拿到，立刻吊销，使这枚凭据不可用。
///
/// 吊销是干净的回滚——同名再次 `type-issue` 会轮换新 token 并解除吊销，
/// 因此回滚之后不存在需要人工清理的半截状态。
fn rollback(store: &AgentStore, agent_type: &str, error: terminal::TerminalError) -> String {
    match store.revoke_type(agent_type, &now()) {
        Ok(_) => format!(
            "token 已签发但显示失败：{error}。\n\
             本次签发已回滚：类型 {agent_type} 已标记吊销，没有任何可用 token。\n\
             修好终端后在交互终端重新执行 type-issue 即可恢复。"
        ),
        Err(revoke_error) => format!(
            "token 已签发但显示失败：{error}；回滚吊销同样失败：{revoke_error}。\n\
             类型 {agent_type} 可能持有一枚主人未取得的有效 token。\n\
             请立刻执行 `studiolo-agent admin type-revoke --type {agent_type}`，\
             再用 `admin types` 核对状态。"
        ),
    }
}
