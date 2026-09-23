//! 策略端点 POST 正文的解析：拒绝未知字段，逐项校验后才得到已解析的 `Input`。

use serde::Deserialize;
use studiolo_policy::{PolicyKind, RevocationLevel};

use super::{ACTIVATE, DESIRED, REVOKE, VALIDATE, WITHDRAW};
use crate::auth_http::responses::Failure;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredBody {
    kind: String,
    subject: String,
    /// 必须显式出现：字符串＝正文，`null`＝撤销；缺省即拒绝，漏写字段不能变成撤销请求。
    #[serde(default, deserialize_with = "explicit")]
    body: Option<Option<String>>,
}

fn explicit<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DesiredRef {
    desired_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateBody {
    #[serde(default)]
    desired_id: Option<String>,
    #[serde(default)]
    rollback_to: Option<u64>,
    #[serde(default)]
    release_global_revocation: Option<bool>,
    expected_effective_hash: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeBody {
    level: String,
    #[serde(default)]
    target: Option<String>,
    reason: String,
}

/// 发布请求的三种形态之一。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActivateRequest {
    /// 发布一份 desired。
    Desired(String),
    /// 回滚到某个版本。
    Rollback(u64),
    /// 解除全局吊销。
    ReleaseGlobal,
}

/// 已解析的策略端点输入。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Submit {
        kind: PolicyKind,
        subject: String,
        body: Option<String>,
    },
    Validate {
        desired_id: String,
    },
    Withdraw {
        desired_id: String,
    },
    Activate {
        request: ActivateRequest,
        expected_effective_hash: String,
    },
    Revoke {
        level: RevocationLevel,
        target: String,
        reason: String,
    },
}

fn valid_id(value: &str) -> bool {
    value.len() == 26 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub(crate) fn parse(path: &str, bytes: &[u8]) -> Result<Input, Failure> {
    let bad = |_| Failure::bad_request();
    match path {
        DESIRED => {
            let body: DesiredBody = serde_json::from_slice(bytes).map_err(bad)?;
            let text = body.body.ok_or_else(Failure::bad_request)?;
            Ok(Input::Submit {
                kind: PolicyKind::parse(&body.kind).map_err(|_| Failure::bad_request())?,
                subject: body.subject,
                body: text,
            })
        }
        VALIDATE | WITHDRAW => {
            let body: DesiredRef = serde_json::from_slice(bytes).map_err(bad)?;
            if !valid_id(&body.desired_id) {
                return Err(Failure::bad_request());
            }
            Ok(if path == VALIDATE {
                Input::Validate {
                    desired_id: body.desired_id,
                }
            } else {
                Input::Withdraw {
                    desired_id: body.desired_id,
                }
            })
        }
        ACTIVATE => {
            let body: ActivateBody = serde_json::from_slice(bytes).map_err(bad)?;
            if body.expected_effective_hash.len() != 64
                || !body
                    .expected_effective_hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(Failure::bad_request());
            }
            let request = match (
                body.desired_id,
                body.rollback_to,
                body.release_global_revocation,
            ) {
                (Some(id), None, None) if valid_id(&id) => ActivateRequest::Desired(id),
                (None, Some(version), None) if version > 0 => ActivateRequest::Rollback(version),
                (None, None, Some(true)) => ActivateRequest::ReleaseGlobal,
                _ => return Err(Failure::bad_request()),
            };
            Ok(Input::Activate {
                request,
                expected_effective_hash: body.expected_effective_hash,
            })
        }
        REVOKE => {
            let body: RevokeBody = serde_json::from_slice(bytes).map_err(bad)?;
            let level = RevocationLevel::parse(&body.level).ok_or_else(Failure::bad_request)?;
            let target = match (level, body.target) {
                (RevocationLevel::Global, None) => "*".to_owned(),
                (RevocationLevel::Global, Some(_)) => return Err(Failure::bad_request()),
                (_, Some(target)) if !target.is_empty() && target.len() <= 64 => target,
                _ => return Err(Failure::bad_request()),
            };
            if body.reason.trim().is_empty()
                || body.reason.chars().count() > 200
                || body.reason.chars().any(char::is_control)
            {
                return Err(Failure::bad_request());
            }
            Ok(Input::Revoke {
                level,
                target,
                reason: body.reason,
            })
        }
        _ => Err(Failure::bad_request()),
    }
}
