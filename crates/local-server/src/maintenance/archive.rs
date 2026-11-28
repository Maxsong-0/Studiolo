//! 写者旧记录的离线打包与还原（BUG-051、T-235）：和在线打包同一套步骤，只是由操作者在停服窗口里发起。
//! 不改正文、回执和账本；断在中间散件和段都在，重跑同一条命令续做。
use super::{MaintenanceError, inputs::Inputs, workflow::guard};
use serde_json::{Value, json};
use studiolo_vault_broker::{ArchiveBatch, ArchiveCursor, BrokerError, MaintenanceActor};

pub(super) fn pack(
    inputs: &mut Inputs,
    actor: &MaintenanceActor,
    confirm: &mut impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    pack_at(inputs, actor, now_ms()?, confirm, check)
}

/// 同上，「现在」由调用方给；生产只经上面那个入口用真实时钟。
pub(super) fn pack_at(
    inputs: &mut Inputs,
    actor: &MaintenanceActor,
    now_ms: i64,
    confirm: &mut impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    let ledger = inputs.audit.as_deref().ok_or(MaintenanceError::Audit)?;
    let seal = crate::backup::queue::seal_mark(ledger)
        .map_err(|_| MaintenanceError::Audit)?
        .ok_or(MaintenanceError::ArchiveNoSeal)?;
    let before = inputs.broker.record_usage().map_err(fault)?;
    guard(inputs, check)?;
    confirm(
        &json!({"command":"archive-pack","operator":actor,"usage":before,"sealed_offset":seal.offset,
            "sealed_content_ms":seal.content_cutoff_ms,
            "note":"把满 7 天、已在最近一次成功封存之内的旧记录整组打包：有审计主体的正文按两份回执的账本位置判，没有的（看板投影）按请求时间早于封存的内容截止时刻判；还有落定的文件操作（改名、删除、恢复、取消、拒绝，终态未知的不打）。散件到 95% 时不满 7 天的也打，没进过成功封存的一条不碰。一条不丢，旧键照答原回执。打过包的库旧程序会拒绝打开，退回旧程序之前先跑 archive-unpack。"}),
        "archive-pack",
    )?;
    let cutoff = seal.cutoff(now_ms);
    let mut cursor = ArchiveCursor::default();
    let mut packed = 0;
    loop {
        // 每一步都重核批准根和终端；断在步与步之间没有半成品。也是凑够一段才写，只有最后剩下的可以是小段。
        guard(inputs, check)?;
        let step = inputs
            .broker
            .archive_step(cutoff, ArchiveBatch::All, &mut cursor)
            .map_err(fault)?;
        packed += step.packed;
        if step.done {
            break;
        }
    }
    guard(inputs, check)?;
    let after = inputs.broker.record_usage().map_err(fault)?;
    Ok(json!({"command":"archive-pack","packed":packed,"sealed_offset":seal.offset,"usage":after}))
}

pub(super) fn unpack(
    inputs: &mut Inputs,
    actor: &MaintenanceActor,
    confirm: &mut impl FnMut(&Value, &str) -> Result<(), MaintenanceError>,
    check: &impl Fn() -> Result<(), MaintenanceError>,
) -> Result<Value, MaintenanceError> {
    let preview = inputs.broker.archive_unpack_preview().map_err(fault)?;
    guard(inputs, check)?;
    confirm(
        &json!({"command":"archive-unpack","operator":actor,"after":preview,
            "note":"把打包的记录全部还原成散件、写者状态写回第 8 版、删掉段文件。over_limit 为真时旧程序起得来、旧键答得出，但新写入和封存会被旧上限拒绝。"}),
        "archive-unpack",
    )?;
    guard(inputs, check)?;
    let done = inputs.broker.archive_unpack().map_err(fault)?;
    guard(inputs, check)?;
    Ok(json!({"command":"archive-unpack","restored":done}))
}

fn fault(error: BrokerError) -> MaintenanceError {
    match error {
        BrokerError::MutationUnresolved => MaintenanceError::ArchiveBusy,
        _ => MaintenanceError::Archive,
    }
}

fn now_ms() -> Result<i64, MaintenanceError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .ok_or(MaintenanceError::Broker)
}
