//! 备份审批的受控 JSON 文档与校验；默认关闭，任何未知字段都失败关闭。

use super::types::{
    BackupTargetShape, CredentialReference, ResticPin, RetentionPolicy, ScheduleBudget,
};
use crate::ConfigError;
use serde::{Deserialize, Deserializer};

/// 本实现接受的唯一 restic 包名。
pub(super) const EXPECTED_PACKAGE: &str = "restic";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialDocument {
    logical_name: String,
    environment_variable: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    target_shape: String,
    /// 主人声明目标具备不可变保护；这只是待核验的主张。
    #[serde(default)]
    claims_immutability: bool,
    restic_package: String,
    restic_version: String,
    schedule_interval_millis: u64,
    completion_budget_millis: u64,
    rpo_millis: u64,
    keep_last: u32,
    keep_daily: u32,
    credentials: Vec<CredentialDocument>,
}

/// 已校验的备份审批。没有这份审批就等于备份未启用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupApproval {
    target_shape: BackupTargetShape,
    claims_immutability: bool,
    restic: ResticPin,
    schedule: ScheduleBudget,
    retention: RetentionPolicy,
    credentials: Vec<CredentialReference>,
}

impl BackupApproval {
    /// 目标形态。
    #[must_use]
    pub fn target_shape(&self) -> BackupTargetShape {
        self.target_shape
    }

    /// 主人是否声明目标具备不可变保护。
    ///
    /// 界面**不得**据此显示「具备不可变保护」：那要由运行侧对真实目标取证。
    #[must_use]
    pub fn claims_immutability(&self) -> bool {
        self.claims_immutability
    }

    /// 固定 restic 包。
    #[must_use]
    pub fn restic(&self) -> &ResticPin {
        &self.restic
    }

    /// 调度与完成预算。
    #[must_use]
    pub fn schedule(&self) -> ScheduleBudget {
        self.schedule
    }

    /// 保留策略。
    #[must_use]
    pub fn retention(&self) -> RetentionPolicy {
        self.retention
    }

    /// 凭据引用；只有变量名。
    #[must_use]
    pub fn credentials(&self) -> &[CredentialReference] {
        &self.credentials
    }

    fn validate(document: Document) -> Result<Self, ConfigError> {
        if document.version != 1 {
            return Err(ConfigError);
        }
        let target_shape = BackupTargetShape::parse(&document.target_shape).ok_or(ConfigError)?;
        // 普通 SMB/NFS 目录与本机临时仓库不可能提供不可变保护；
        // 允许这种声明落盘，就等于允许界面显示一个假的绿灯。
        if document.claims_immutability && !target_shape.may_claim_immutability() {
            return Err(ConfigError);
        }
        if document.restic_package != EXPECTED_PACKAGE || !is_version(&document.restic_version) {
            return Err(ConfigError);
        }
        let schedule = ScheduleBudget {
            interval_millis: document.schedule_interval_millis,
            completion_millis: document.completion_budget_millis,
            rpo_millis: document.rpo_millis,
        };
        if schedule.interval_millis == 0
            || schedule.completion_millis == 0
            || schedule.rpo_millis == 0
            || !schedule.satisfies_rpo()
        {
            return Err(ConfigError);
        }
        if document.keep_last == 0 || document.keep_daily == 0 {
            return Err(ConfigError);
        }
        if document.credentials.is_empty() || document.credentials.len() > 8 {
            return Err(ConfigError);
        }
        let mut credentials = Vec::with_capacity(document.credentials.len());
        for credential in document.credentials {
            if !is_logical_name(&credential.logical_name)
                || !is_environment_variable(&credential.environment_variable)
            {
                return Err(ConfigError);
            }
            credentials.push(CredentialReference {
                logical_name: credential.logical_name,
                environment_variable: credential.environment_variable,
            });
        }
        Ok(Self {
            target_shape,
            claims_immutability: document.claims_immutability,
            restic: ResticPin {
                package: document.restic_package,
                version: document.restic_version,
            },
            schedule,
            retention: RetentionPolicy {
                keep_last: document.keep_last,
                keep_daily: document.keep_daily,
            },
            credentials,
        })
    }
}

fn is_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
}

fn is_logical_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// 只接受变量名形状。含 `=`、空白或小写字母的写法很可能是把值贴了进来。
fn is_environment_variable(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with(|character: char| character.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

/// 供组合根接线：`#[serde(default, deserialize_with = "backup::decode")]`。
///
/// root 只需在 `Config` 的 `Document` 里加一行 `backup` 字段并指向本函数；
/// 缺字段即视为备份未启用。
pub fn decode<'de, D: Deserializer<'de>>(input: D) -> Result<Option<BackupApproval>, D::Error> {
    struct Object;
    impl<'de> serde::de::Visitor<'de> for Object {
        type Value = Option<BackupApproval>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("备份审批对象")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            let document =
                Document::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
            BackupApproval::validate(document)
                .map(Some)
                .map_err(serde::de::Error::custom)
        }
    }
    input.deserialize_map(Object)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
