//! 可信本地管理子命令：初始化私有库与账本、签发/吊销类型 token、读取审计查询、索引重建、outage 列表。

use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_supply::{AgentStore, ReadQuery, ReadRecord, audit::CONTENT_READ};

use crate::{options::AdminCommand, state::now};

fn parse_time(value: &str) -> Result<i64, String> {
    value
        .parse::<jiff::Timestamp>()
        .map(|timestamp| timestamp.as_millisecond())
        .map_err(|_| format!("时间必须是 RFC 3339：{value}"))
}

/// 执行一个管理子命令；秘密只打印一次到 stdout。
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
        AdminCommand::TypeIssue { data, agent_type } => {
            let store = AgentStore::open_existing(&data).map_err(|error| error.to_string())?;
            let issued = store
                .issue_type(&agent_type, &now())
                .map_err(|error| error.to_string())?;
            println!(
                "类型 {} 的 token（只显示这一次，库中只存摘要）：\n{}",
                issued.agent_type, issued.token
            );
            Ok(())
        }
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
