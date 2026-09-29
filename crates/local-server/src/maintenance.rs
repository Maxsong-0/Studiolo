//! 离线事务维护：使用实际 OS 权限和 Broker 独占租约，不构造应用身份。

mod audit;
mod flush;
mod inputs;
mod options;
mod presentation;
mod rename;
mod state;
mod sync;
mod sync_audit;
mod sync_rebind;
mod terminal;
mod workflow;

use std::io::Write;

/// 本机显式入口；不读取任何秘密环境，不开启监听或停止已有服务。
///
/// # Errors
///
/// 秘密驻留面收紧或其记录失败即 [`MaintenanceError::Process`] /
/// [`MaintenanceError::ProcessReport`]，此时未打开任何写者存储；其余失败见各变体。
pub fn run() -> Result<(), MaintenanceError> {
    // 与登记工具、服务器同一份实现、同一套失败语义：本平台提供的能力必须真正生效
    // 并回读确认，否则整体拒绝且不打开任何存储；本平台不提供的能力记为明确的
    // 「不支持」，并随逐项分类一起记录到标准错误（JSON 结果仍独占标准输出）。
    // 位置在参数解析之前：原 proposal 与维护票据都可能被 Broker 读进本进程内存。
    let residency = studiolo_process_safety::harden()?;
    writeln!(std::io::stderr(), "秘密驻留面：{residency}")
        .map_err(|_| MaintenanceError::ProcessReport)?;
    let Some(options) = options::parse(std::env::args_os().skip(1).take(13).collect())? else {
        return output(&serde_json::json!({"usage":[
            "studiolo-maintain inspect --config /owner.json --vault /vault --assets /assets --writer-data /writer",
            "studiolo-maintain cancel-prepared|preserve-installing|cancel-restore-intent [相同根参数] --request <64位审阅指纹>",
            "studiolo-maintain cancel-maintenance [相同根参数] --maintenance <64位维护指纹>",
            "studiolo-maintain audit-flush [相同根参数]",
            "studiolo-maintain sync-init [相同根参数] --sync-transport </绝对路径/sync-approval.json>",
            "studiolo-maintain sync-rebind [相同根参数] --sync-transport </绝对路径/新sync-approval.json>",
            "studiolo-maintain sync-purge-residue [相同根参数] --sync-transport </绝对路径/sync-approval.json> --folder <排除文件夹名>"
        ],"boundary":"仅本机 OS 权限与既有 Broker owner 租约；不是应用身份认证，不自动初始化、取消或停服务。sync-init 只在四项私有存储都不存在时显式建立；sync-purge-residue 须先停 studiolo-human，只删镜像视图内已不再排除的文件夹残留。"}));
    };
    let terminal = terminal::Terminal::open()?;
    let operator = terminal.operator();
    let actor = studiolo_vault_broker::MaintenanceActor::LocalOs {
        uid: operator.uid,
        gid: operator.gid,
    };
    let mut inputs = inputs::Inputs::open(&options)?;
    let report = workflow::execute(
        &mut inputs,
        &options,
        actor,
        |review, expected| {
            terminal.review(review)?;
            terminal.confirm(expected)
        },
        || terminal.revalidate(),
    )?;
    terminal.revalidate()?;
    inputs.revalidate()?;
    output(&report)
}

fn output(value: &serde_json::Value) -> Result<(), MaintenanceError> {
    // 所有逻辑路径都经 JSON 转义，不把路径里的终端控制字符原样打印。
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|_| MaintenanceError::Output)?;
    writeln!(out).map_err(|_| MaintenanceError::Output)?;
    out.flush().map_err(|_| MaintenanceError::Output)
}

/// 固定脱敏错误；不输出路径、数据库诊断、配置正文或恢复材料。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MaintenanceError {
    /// 原 proposal 可能由 Broker 读进内存，当前短命进程须先收紧秘密驻留面。
    ///
    /// 四项措施与失败语义同登记工具与服务器，实现只有
    /// `studiolo_process_safety` 一份，这里不自持部分收紧。
    #[error("{0}；未打开写者存储")]
    Process(#[from] studiolo_process_safety::Refusal),
    /// 逐项分类必须被操作者看见；记录不下来就等于静默跳过，同样不打开存储。
    #[error("无法记录秘密驻留面逐项分类；未打开写者存储")]
    ProcessReport,
    /// 参数闭集不匹配。
    #[error("维护参数无效；使用 --help 查看固定命令")]
    Arguments,
    /// 当前进程不是同一前台控制终端上的非提权调用。
    #[error("需要实际 OS 用户、非 setuid/setgid 进程及同一前台控制终端")]
    Terminal,
    /// 明确确认缺失、超时或不匹配。
    #[error("维护确认已取消、超时或不匹配；未据此开始决定")]
    Confirmation,
    /// 配置、根身份或分离关系失效。
    #[error("私有配置或既有根审批失败")]
    Configuration,
    /// Broker 独占权或稳定的未决状态不可用。
    #[error("既有写者存储不可用、已有 owner 或维护票据已失效")]
    Broker,
    /// 审计失败不能被当作维护完成。
    #[error("维护审计未确认；必须检查状态并完成 audit-flush，不能推断原写入成功")]
    Audit,
    /// 已执行的决定未取得可验证结论。
    #[error("维护结果未确认；请重新 inspect，不得按内容哈希推断成功")]
    OutcomeUnknown,
    /// JSON 输出失败不回显底层错误。
    #[error("维护结果输出失败；请重新 inspect 核验持久状态")]
    Output,
    /// Bridge 平面四项私有存储里已有实例；本命令不修复、不覆盖。
    #[error("Bridge 平面私有存储已存在；请先人工处理，本命令不修复、不覆盖")]
    SyncExists,
    /// Bridge 平面私有存储初始化或打开失败。
    #[error("Bridge 平面私有存储不可用；初始化或读取未完成，请检查四个根后重试")]
    SyncStorage,
    /// 重绑定前置拒绝，不复述路径、原配置或底层日志。
    #[error("传输重绑定拒绝：{0}；未发布新绑定")]
    SyncRebind(studiolo_sync_transport::owned_run::TransportError),
    /// 平面有未回收运行或 Bridge 未闭合批次，必须先按原键恢复。
    #[error("平面仍有未回收运行、未闭合 Bridge 批次或待补审计；先按原键恢复，未发布新绑定")]
    SyncRebindPending,
    /// 原子发布已尝试，不能将后续核验错误描述成没有写入。
    #[error("重绑定发布结果未确认；请重开核验持久配置摘要与代次，不得据此重复初始化")]
    SyncRebindOutcome,
    /// 指定文件夹不是当前可清除的排除残留。
    #[error("指定文件夹不在上次核验的排除集合内，或仍在当前审批的排除清单里；未删除任何东西")]
    SyncResidue,
    /// headless 最近一次核对已不再排除该文件夹：此时清残留，下一次 RunOnce 可能把镜像内容当新内容上传。
    #[error(
        "headless 最近一次核对已不再排除该文件夹，先在 headless 侧恢复排除、让 human 跑一轮核对后再清残留；未删除任何东西"
    )]
    SyncResidueHeadless,
    /// 残留里有链接、非常规对象，或超出遍历预算。
    #[error("镜像残留含链接、非常规对象或超出遍历预算；未删除任何东西")]
    SyncResidueUnsafe,
}

#[cfg(test)]
#[path = "../tests/maintenance/mod.rs"]
mod tests;
