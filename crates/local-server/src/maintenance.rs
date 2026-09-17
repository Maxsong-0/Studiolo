//! 离线事务维护：使用实际 OS 权限和 Broker 独占租约，不构造应用身份。

mod audit;
mod flush;
mod inputs;
mod options;
mod presentation;
mod rename;
mod state;
mod terminal;
mod workflow;

use std::io::Write;

/// 本机显式入口；不读取任何秘密环境，不开启监听或停止已有服务。
pub fn run() -> Result<(), MaintenanceError> {
    let Some(options) = options::parse(std::env::args_os().skip(1).take(13).collect())? else {
        return output(&serde_json::json!({"usage":[
            "studiolo-maintain inspect --config /owner.json --vault /vault --assets /assets --writer-data /writer",
            "studiolo-maintain cancel-prepared|preserve-installing|cancel-restore-intent [相同根参数] --request <64位审阅指纹>",
            "studiolo-maintain cancel-maintenance [相同根参数] --maintenance <64位维护指纹>",
            "studiolo-maintain audit-flush [相同根参数]"
        ],"boundary":"仅本机 OS 权限与既有 Broker owner 租约；不是应用身份认证，不自动初始化、取消或停服务。"}));
    };
    let disabled = rustix::process::Rlimit {
        current: Some(0),
        maximum: Some(0),
    };
    rustix::process::setrlimit(rustix::process::Resource::Core, disabled)
        .map_err(|_| MaintenanceError::Process)?;
    let actual = rustix::process::getrlimit(rustix::process::Resource::Core);
    if actual.current != Some(0) || actual.maximum != Some(0) {
        return Err(MaintenanceError::Process);
    }
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
    /// 原 proposal 可能由 Broker 读取，当前短命进程须先关闭核心转储。
    #[error("无法关闭当前维护进程的核心转储；未打开写者存储")]
    Process,
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
}

#[cfg(test)]
#[path = "../tests/maintenance/mod.rs"]
mod tests;
