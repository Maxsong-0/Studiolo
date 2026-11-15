//! 出站事件门铃的调用入口（ADR-033）。缺省只预演；定时器显式 `--deliver` 才读发送配置、
//! 读审计账本并投递。一次运行做完就退出，重试靠定时器下一次拉起。

use std::{
    io::Write,
    path::Path,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_observability::{
    doorbell::{HttpEndpoint, RunError, RunOutcome, RunSummary, run},
    store::DoorbellStore,
};

#[derive(Debug, PartialEq, Eq)]
struct Request {
    audit: String,
    state: String,
    deliver: bool,
}

fn parse(args: &[String]) -> Result<Request, &'static str> {
    if args.len() != 2 && args.len() != 3 {
        return Err("参数应为私有审计目录、私有状态目录及可选 --deliver");
    }
    for directory in &args[..2] {
        if !Path::new(directory).is_absolute() || directory.len() > 4096 {
            return Err("审计目录与状态目录都必须是绝对路径");
        }
    }
    if args[0] == args[1] {
        return Err("状态目录不能和审计目录是同一个");
    }
    let deliver = match args.get(2).map(String::as_str) {
        None => false,
        Some("--deliver") => true,
        _ => return Err("发送开关无效"),
    };
    Ok(Request {
        audit: args[0].clone(),
        state: args[1].clone(),
        deliver,
    })
}

/// 一次运行的结论变成一行输出；需要人来处理的算失败，等一等就好的不算。
fn report(summary: RunSummary) -> Result<String, &'static str> {
    match summary.outcome {
        RunOutcome::Enabled => Ok("enabled".into()),
        RunOutcome::CaughtUp => Ok(format!(
            "caught_up delivered={} expired={}",
            summary.delivered, summary.expired
        )),
        RunOutcome::Backoff => Ok("backoff".into()),
        RunOutcome::Deferred(classification) => Ok(format!(
            "deferred {classification} delivered={} expired={}",
            summary.delivered, summary.expired
        )),
        RunOutcome::EndpointGone => Err("收件方已停用这个地址（410）；换收件地址后恢复"),
    }
}

fn refusal(error: RunError) -> &'static str {
    match error {
        RunError::NotConfigured => "收件端未配置",
        RunError::Malformed => "收件配置不合法",
        RunError::StateUnavailable => "门铃状态不可用或已损坏；核对后再处理，不会自动从头开始",
        RunError::AuditUnavailable => "审计不可用；没有记进审计的事件不会发出",
    }
}

fn execute() -> Result<String, &'static str> {
    let args = std::env::args_os()
        .skip(1)
        .take(4)
        .map(|value| value.into_string().map_err(|_| "参数编码无效"))
        .collect::<Result<Vec<_>, _>>()?;
    let request = parse(&args)?;
    if !request.deliver {
        return Ok("dry_run_no_delivery".into());
    }
    let residency = studiolo_process_safety::harden().map_err(|_| "秘密驻留面加固失败")?;
    writeln!(std::io::stderr(), "秘密驻留面：{residency}").map_err(|_| "加固回执不可写")?;
    // 只开已有的账本和状态目录；门铃不创建、不修复任何一个。
    let ledger = SqliteAuditLedger::open_existing(Path::new(&request.audit))
        .map_err(|_| "私有审计库不可用")?;
    let store = DoorbellStore::open(Path::new(&request.state)).map_err(|_| "私有状态目录不可用")?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .ok_or("系统时钟不可用")?;
    let summary =
        run(&ledger, &store, &HttpEndpoint::from_environment(), now_ms).map_err(refusal)?;
    report(summary)
}

fn main() -> ExitCode {
    match execute() {
        Ok(status) => {
            println!("{status}");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("门铃调用失败：{reason}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn delivery_needs_the_explicit_switch_and_two_distinct_absolute_directories() {
        let dry = parse(&args(&["/private/audit", "/private/doorbell"])).unwrap();
        assert!(!dry.deliver);
        let live = parse(&args(&["/private/audit", "/private/doorbell", "--deliver"])).unwrap();
        assert!(
            live.deliver && live.audit == "/private/audit" && live.state == "/private/doorbell"
        );
        for refused in [
            args(&[]),
            args(&["/private/audit"]),
            args(&["relative/audit", "/private/doorbell"]),
            args(&["/private/audit", "relative/doorbell"]),
            args(&["/private/audit", "/private/audit"]),
            args(&["/private/audit", "/private/doorbell", "--send"]),
            args(&["/private/audit", "/private/doorbell", "--deliver", "extra"]),
        ] {
            assert!(parse(&refused).is_err(), "不该接受：{refused:?}");
        }
    }

    #[test]
    fn only_outcomes_that_need_a_person_fail_the_unit() {
        let summary = |outcome| RunSummary {
            outcome,
            delivered: 2,
            expired: 1,
        };
        assert_eq!(report(summary(RunOutcome::Enabled)).unwrap(), "enabled");
        assert_eq!(
            report(summary(RunOutcome::CaughtUp)).unwrap(),
            "caught_up delivered=2 expired=1"
        );
        assert_eq!(report(summary(RunOutcome::Backoff)).unwrap(), "backoff");
        assert_eq!(
            report(summary(RunOutcome::Deferred("timeout"))).unwrap(),
            "deferred timeout delivered=2 expired=1"
        );
        assert!(report(summary(RunOutcome::EndpointGone)).is_err());
    }
}
