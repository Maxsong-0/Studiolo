//! 秘密字段一经反序列化取得所有权即进入Auth零化封装，后续JSON失败也会清理。

use serde::{Deserialize, Deserializer, de::Error};
use studiolo_auth::{RecoveryCode, TotpCode};

use super::{RECOVER, RECOVERY_CODES, STEP_UP, responses::Failure};

pub(super) enum Action {
    StepUp(TotpCode),
    RotateCodes,
    Recover {
        recovery_code: RecoveryCode,
        code: TotpCode,
    },
}

struct Code(TotpCode);
struct Recovery(RecoveryCode);

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TotpCode::parse(String::deserialize(deserializer)?)
            .map(Self)
            .map_err(|_| D::Error::custom("验证码格式无效"))
    }
}

impl<'de> Deserialize<'de> for Recovery {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        RecoveryCode::parse(String::deserialize(deserializer)?)
            .map(Self)
            .map_err(|_| D::Error::custom("恢复码格式无效"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepUp {
    code: Code,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recover {
    recovery_code: Recovery,
    code: Code,
}

pub(super) fn parse(path: &str, bytes: &[u8]) -> Result<Action, Failure> {
    match path {
        STEP_UP => {
            serde_json::from_slice::<StepUp>(bytes).map(|value| Action::StepUp(value.code.0))
        }
        RECOVERY_CODES => serde_json::from_slice::<Empty>(bytes).map(|_| Action::RotateCodes),
        RECOVER => serde_json::from_slice::<Recover>(bytes).map(|value| Action::Recover {
            recovery_code: value.recovery_code.0,
            code: value.code.0,
        }),
        _ => return Err(Failure::bad_request()),
    }
    .map_err(|_| Failure::bad_request())
}
