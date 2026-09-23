//! 策略平面端点的输入解析与主体提取：L2 读写与 L3 发布共用同一条会话链。
//!
//! 只在这里出现路径常量与方法表；`gate`、`security`、`input` 与 `policy_http` 都从这里取，
//! 新增端点漏改一侧即门禁旁路的老毛病不再有第二份表可漏（T-026 复审 #22 的策略端点部分）。

mod principal;
mod request;

pub(crate) use principal::{L3Denial, PolicyPrincipal};
pub(super) use request::parse;
pub(crate) use request::{ActivateRequest, Input};

/// effective 摘要与生效中的吊销。
pub(crate) const STATUS: &str = "/api/v1/policy";
/// GET 列 desired；POST 提交 desired。
pub(crate) const DESIRED: &str = "/api/v1/policy/desired";
/// 复核一份 desired。
pub(crate) const VALIDATE: &str = "/api/v1/policy/validate";
/// 撤回一份 desired。
pub(crate) const WITHDRAW: &str = "/api/v1/policy/withdraw";
/// 发布／回滚／解除全局吊销（L3）。
pub(crate) const ACTIVATE: &str = "/api/v1/policy/activate";
/// 版本列表；`/api/v1/policy/versions/{n}` 读某版正文。
pub(crate) const VERSIONS: &str = "/api/v1/policy/versions";
/// 三级吊销（L2）。
pub(crate) const REVOKE: &str = "/api/v1/agents/revoke";
/// 单份 desired 的 JSON 正文上限（规则母本 256 KiB ＋ 转义余量）。
pub(crate) const JSON_LIMIT: usize = 512 * 1024;

/// 策略端点的方法表：单一来源。
pub(crate) fn method_for(path: &str) -> Option<&'static str> {
    match path {
        STATUS | VERSIONS => Some("GET"),
        DESIRED => Some("GET, POST"),
        VALIDATE | WITHDRAW | ACTIVATE | REVOKE => Some("POST"),
        _ if path
            .strip_prefix("/api/v1/policy/versions/")
            .is_some_and(|rest| {
                !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
            }) =>
        {
            Some("GET")
        }
        _ => None,
    }
}

pub(crate) fn is_policy_path(path: &str) -> bool {
    method_for(path).is_some()
}

pub(crate) fn is_policy_post(path: &str) -> bool {
    matches!(path, DESIRED | VALIDATE | WITHDRAW | ACTIVATE | REVOKE)
}

/// 只有发布需要 L3，因而只有它需要可信 Unix 来源证明。
pub(crate) fn requires_l3(path: &str) -> bool {
    path == ACTIVATE
}
