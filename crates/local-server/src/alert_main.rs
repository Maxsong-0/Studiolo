//! 带外发送的真实调用入口。缺省只预演；定时器显式 --deliver 才读取发送凭据与发送。
//! 状态机调用方先持久化 episode/round 并串行调用，本程序负责审计门槛和同轮回执核对。

use std::{io::Write, path::Path, process::ExitCode};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};
use studiolo_observability::alert::{
    AlertDispatcher, AlertMessage, AlertSeverity, DispatchOutcome, IdempotencyKey, PushTransport,
    dispatch_event_id,
};

struct Request {
    directory: String,
    message: AlertMessage,
    round: u64,
    deliver: bool,
}

fn parse(args: &[String]) -> Result<Request, &'static str> {
    if args.len() != 5 && args.len() != 6 {
        return Err("参数应为私有审计目录、等级、代号、情节号、轮次及可选 --deliver");
    }
    if !Path::new(&args[0]).is_absolute() || args[0].len() > 4096 {
        return Err("私有审计目录必须是绝对路径");
    }
    let severity = match args[1].as_str() {
        "warning" => AlertSeverity::Warning,
        "critical" => AlertSeverity::Critical,
        _ => return Err("告警等级无效"),
    };
    let code = match args[2].as_str() {
        "A01" => "A01",
        "A02" => "A02",
        "A03" => "A03",
        "A04" => "A04",
        "A06" => "A06",
        _ => return Err("告警代号无效"),
    };
    let episode: u64 = args[3].parse().map_err(|_| "情节号无效")?;
    let round = args[4].parse().map_err(|_| "投递轮次无效")?;
    let deliver = match args.get(5).map(String::as_str) {
        None => false,
        Some("--deliver") => true,
        _ => return Err("发送开关无效"),
    };
    let key = IdempotencyKey::derive("operator-watchdog", &[code, &episode.to_string()]);
    let message =
        AlertMessage::new(key, severity, code, "需要查看运行状态").map_err(|_| "告警消息无效")?;
    Ok(Request {
        directory: args[0].clone(),
        message,
        round,
        deliver,
    })
}

fn send(
    request: &Request,
    ledger: &dyn AuditLedger,
    transport: &dyn studiolo_observability::alert::AlertTransport,
) -> Result<&'static str, &'static str> {
    let event = dispatch_event_id(&request.message, request.round).map_err(|_| "审计标识不可用")?;
    let records = ledger
        .query(&AuditFilter::default().with_event_id(event))
        .map_err(|_| "审计不可用")?;
    if records.iter().any(|record| {
        record.event().phase() == AuditPhase::Outcome
            && record.event().payload().as_value()["classification"] == "delivered"
    }) {
        return Ok("already_accepted");
    }
    // 已开始但没得到成功回执的同轮结果可能不确定；不能覆写旧审计或假称这轮未发送。
    if !records.is_empty() {
        return Err("这一轮已记录但未确认接受；核对后以新轮次重试");
    }
    let mut dispatcher = AlertDispatcher::new(1, 1).map_err(|_| "调度参数不可用")?;
    match dispatcher.dispatch(ledger, transport, &request.message, request.round) {
        DispatchOutcome::Delivered { .. } => Ok("accepted"),
        DispatchOutcome::AlreadyDelivered => Ok("already_accepted"),
        DispatchOutcome::NotConfigured => Err("发送端未配置"),
        DispatchOutcome::AuditUnavailable => Err("审计不可用；投递结果须核对"),
        DispatchOutcome::Failed { classification, .. } => Err(classification),
    }
}

fn run() -> Result<&'static str, &'static str> {
    let args = std::env::args_os()
        .skip(1)
        .take(8)
        .map(|value| value.into_string().map_err(|_| "参数编码无效"))
        .collect::<Result<Vec<_>, _>>()?;
    let request = parse(&args)?;
    if !request.deliver {
        return Ok("dry_run_no_delivery");
    }
    let residency = studiolo_process_safety::harden().map_err(|_| "秘密驻留面加固失败")?;
    writeln!(std::io::stderr(), "秘密驻留面：{residency}").map_err(|_| "加固回执不可写")?;
    let ledger =
        SqliteAuditLedger::open(Path::new(&request.directory)).map_err(|_| "私有审计库不可用")?;
    send(&request, &ledger, &PushTransport::from_environment())
}

fn main() -> ExitCode {
    match run() {
        Ok(status) => {
            println!("{status}");
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("告警调用失败：{reason}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use studiolo_audit_ledger::MemoryAuditLedger;
    use studiolo_observability::alert::{AlertTransport, DeliveryReceipt, TransportError};

    struct Fake(AtomicUsize);
    impl AlertTransport for Fake {
        fn deliver(&self, message: &AlertMessage) -> Result<DeliveryReceipt, TransportError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(DeliveryReceipt::new(message.key().clone()))
        }
    }
    #[test]
    fn a_restarted_invocation_reuses_the_successful_round_without_sending_twice() {
        let args = ["/private/audit", "critical", "A01", "7", "9", "--deliver"].map(str::to_owned);
        let request = parse(&args).unwrap();
        let ledger = MemoryAuditLedger::new();
        let fake = Fake(AtomicUsize::new(0));
        assert_eq!(send(&request, &ledger, &fake), Ok("accepted"));
        assert_eq!(
            send(&parse(&args).unwrap(), &ledger, &fake),
            Ok("already_accepted")
        );
        assert_eq!(fake.0.load(Ordering::SeqCst), 1);
        assert!(!parse(&args[..5]).unwrap().deliver);
    }
}
