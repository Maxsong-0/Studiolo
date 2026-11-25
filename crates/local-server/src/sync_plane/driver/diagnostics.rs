//! 运行失败的封闭分类日志；没有路径、正文或任意错误格式化入口。
//! 写日志失败不改变原结果；调用方仍沿用原来的失败关闭与重试协议。
use super::super::PlaneError;
use std::io::Write as _;
use studiolo_sync_bridge::sqlite::BridgeError;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Step {
    Open,
    State,
    Inspect,
    Residue,
    SetMode,
    NextSequence,
    SaveState,
    RunOnce,
    Quiescent,
    Unresolved,
    Observe,
    Plan,
    RetainConflicts,
    DeleteGate,
    Preresolve,
    Preserve,
    Execute,
    Canonical,
    Mirror,
    Approve,
}

/// 删除守卫挡下本轮时留一行固定日志（BUG-046）：不登录健康页也能从服务日志看出同步在等人处理；只有个数，不带路径。
pub(crate) fn deletes_blocked(count: u64) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "studiolo_bridge step=DeleteGate outcome=deletes_blocked count={count}"
    );
}

/// 三方并集超过同步路径预算、这一轮观察停住时留一行固定日志（BUG-047）：只有两个数，不带路径。
pub(crate) fn path_budget_exceeded(used: u64, budget: u64) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "studiolo_bridge step=Observe outcome=path_budget_exceeded used={used} budget={budget}"
    );
}

/// 比对内容超过字节预算、这一轮在批准前停住时留一行固定日志：只有两个数，不带路径。
pub(crate) fn byte_budget_exceeded(used: u64, budget: u64) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "studiolo_bridge step=Approve outcome=byte_budget_exceeded used={used} budget={budget}"
    );
}

/// 上一个进程登记了一次运行、还没发启动就没了：这一轮把它收成「没执行」，留一行固定日志（T-239）。
pub(crate) fn unlaunched_run_cancelled() {
    let _ = writeln!(
        std::io::stderr().lock(),
        "studiolo_bridge component=transport outcome=unlaunched_run_cancelled"
    );
}

pub(crate) fn at<T>(step: Step, result: Result<T, PlaneError>) -> Result<T, PlaneError> {
    if let Err(error) = result.as_ref() {
        let _ = writeln!(
            std::io::stderr().lock(),
            "studiolo_bridge step={step:?} error={error:?}"
        );
    }
    result
}

// BridgeError 为无载荷闭集；新增载荷必须先显式更新本映射，不能隐式泄漏内容。
pub(crate) fn storage(error: BridgeError) -> PlaneError {
    let code = match error {
        BridgeError::MissingStore => "MissingStore",
        BridgeError::AlreadyExists => "AlreadyExists",
        BridgeError::UnsafeStorage => "UnsafeStorage",
        BridgeError::Busy => "Busy",
        BridgeError::UnsupportedPlatform => "UnsupportedPlatform",
        BridgeError::UnsupportedSchema => "UnsupportedSchema",
        BridgeError::CorruptStore => "CorruptStore",
        BridgeError::Unavailable => "Unavailable",
        BridgeError::Budget => "Budget",
        BridgeError::Overflow => "Overflow",
        BridgeError::BindingMismatch => "BindingMismatch",
        BridgeError::VersionConflict => "VersionConflict",
        BridgeError::PendingUnresolved => "PendingUnresolved",
        BridgeError::Fused => "Fused",
        BridgeError::RetryMismatch => "RetryMismatch",
        BridgeError::AuditUnavailable => "AuditUnavailable",
        BridgeError::IntentRequired => "IntentRequired",
        BridgeError::OutcomeUnknown => "OutcomeUnknown",
        BridgeError::OutcomeMismatch => "OutcomeMismatch",
        BridgeError::RetentionRequired => "RetentionRequired",
        BridgeError::Retired => "Retired",
        BridgeError::NotFound => "NotFound",
        BridgeError::NotSettled => "NotSettled",
        BridgeError::StaleObservation => "StaleObservation",
    };
    let _ = writeln!(
        std::io::stderr().lock(),
        "studiolo_bridge component=state error={code}"
    );
    PlaneError::Storage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_preserve_success_and_fail_closed_result() {
        assert_eq!(at(Step::Observe, Ok(37)), Ok(37));
        for error in [
            PlaneError::Unknown,
            PlaneError::Stopping,
            PlaneError::Observation,
            PlaneError::Storage,
            PlaneError::Audit,
            PlaneError::Quiescence,
        ] {
            assert_eq!(at::<()>(Step::Execute, Err(error)), Err(error));
        }
        assert_eq!(storage(BridgeError::VersionConflict), PlaneError::Storage);
    }
}
