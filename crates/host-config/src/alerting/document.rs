//! 有界审批文档解析；默认关闭，未显式启用不产生任何能力。
//!
//! 本模块只做纯数据校验：不打开文件、**不读环境变量**、不解析凭证。
//!
//! **秘密纪律**：收件地址与 SMTP 凭据不在本文档里，也不允许出现在这里——
//! `deny_unknown_fields` 会把任何多余字段直接判非法。真实发送端只经
//! 环境变量 `STUDIOLO_ALERT_SMTP_URL` / `STUDIOLO_ALERT_TO` 注入，
//! 由 `studiolo-observability` 的 transport 读取；仓库里只写"去哪拿"。

use super::types::{DeadmanSchedule, DiskLimits, DispatchLimits, SyncLimits};
use crate::{ConfigError, absolute_normal};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// 审批文档字节上限。
const APPROVAL_MAX_BYTES: usize = 8 * 1024;
const DAY: u64 = 86_400;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u8,
    enabled: bool,
    state_root: PathBuf,
    recovery_point_budget_seconds: u64,
    owners: Vec<String>,
    deadman: DeadmanDocument,
    disk: DiskDocument,
    sync: SyncDocument,
    dispatch: DispatchDocument,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeadmanDocument {
    period_seconds: u64,
    debounce_seconds: u64,
    cooldown_seconds: u64,
    max_suppressed: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiskDocument {
    minimum_free_bytes: u64,
    warn_free_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncDocument {
    max_objects: u64,
    max_bytes: u64,
    warn_objects: u64,
    warn_bytes: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DispatchDocument {
    max_attempts: u32,
    dedupe_capacity: u32,
}

/// 已解析并逐字段校验的健康与告警审批；没有公开构造器或字段。
///
/// 成功只代表这些值形式合法且自洽，**不代表**发送端已配置、
/// 恢复点已建立，或本机真的具备对应的私有目录。
///
/// ```compile_fail
/// let approval: studiolo_host_config::alerting::AlertingApproval =
///     serde_json::from_str("{}").unwrap();
/// ```
pub struct AlertingApproval {
    state_root: PathBuf,
    recovery_point_budget_seconds: u64,
    owners: Vec<String>,
    deadman: DeadmanSchedule,
    disk: DiskLimits,
    sync: SyncLimits,
    dispatch: DispatchLimits,
    digest: [u8; 32],
}

impl AlertingApproval {
    /// 解析主人显式提供的审批文档字节。
    ///
    /// 只接受 schema 1、显式 `enabled = true` 的命名字段对象；未知字段、
    /// 位置数组、尾随对象、超预算输入一律拒绝。任何形似收件地址或凭据的
    /// 多余字段都会因为未知字段被拒，不会被静默忽略。
    pub fn parse(bytes: &[u8]) -> Result<Self, ConfigError> {
        if bytes.len() > APPROVAL_MAX_BYTES {
            return Err(ConfigError);
        }
        let first = bytes
            .iter()
            .copied()
            .find(|byte| !matches!(*byte, b' ' | b'\n' | b'\r' | b'\t'));
        if first != Some(b'{') {
            return Err(ConfigError);
        }
        let document: Document = serde_json::from_slice(bytes).map_err(|_| ConfigError)?;
        if document.schema_version != 1 || !document.enabled {
            return Err(ConfigError);
        }
        if !absolute_normal(&document.state_root)
            || !(60..=90 * DAY).contains(&document.recovery_point_budget_seconds)
        {
            return Err(ConfigError);
        }
        let owners = owners(&document.owners)?;
        let deadman = deadman(&document.deadman)?;
        let disk = disk(&document.disk)?;
        let sync = sync(&document.sync)?;
        let dispatch = dispatch(&document.dispatch)?;
        let mut hash = Sha256::new();
        hash.update(b"studiolo.alerting.approval.v1\0");
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        Ok(Self {
            state_root: document.state_root,
            recovery_point_budget_seconds: document.recovery_point_budget_seconds,
            owners,
            deadman,
            disk,
            sync,
            dispatch,
            digest: hash.finalize().into(),
        })
    }

    /// 死人开关状态文件所在的私有目录；本模块不创建也不打开它。
    #[must_use]
    pub fn state_root(&self) -> &std::path::Path {
        &self.state_root
    }

    /// 恢复点年龄预算。
    #[must_use]
    pub fn recovery_point_budget_seconds(&self) -> u64 {
        self.recovery_point_budget_seconds
    }

    /// 需要活性回执的组件角色；是进程角色，不是自然人身份。
    #[must_use]
    pub fn owners(&self) -> &[String] {
        &self.owners
    }

    /// 死人开关周期参数。
    #[must_use]
    pub fn deadman(&self) -> DeadmanSchedule {
        self.deadman
    }

    /// 磁盘配额水位。
    #[must_use]
    pub fn disk(&self) -> DiskLimits {
        self.disk
    }

    /// Sync 配额。
    #[must_use]
    pub fn sync(&self) -> SyncLimits {
        self.sync
    }

    /// 告警调度预算。
    #[must_use]
    pub fn dispatch(&self) -> DispatchLimits {
        self.dispatch
    }

    /// 审批文档的域分隔摘要；只是绑定事实，不是身份凭据。
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

fn owners(values: &[String]) -> Result<Vec<String>, ConfigError> {
    if values.is_empty() || values.len() > 16 {
        return Err(ConfigError);
    }
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        if value.is_empty()
            || value.len() > 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || !seen.insert(value.as_str())
        {
            return Err(ConfigError);
        }
    }
    Ok(values.to_vec())
}

fn deadman(document: &DeadmanDocument) -> Result<DeadmanSchedule, ConfigError> {
    if !(60..=30 * DAY).contains(&document.period_seconds)
        || document.debounce_seconds > document.period_seconds
        || document.cooldown_seconds > 30 * DAY
        || !(1..=64).contains(&document.max_suppressed)
    {
        return Err(ConfigError);
    }
    Ok(DeadmanSchedule {
        period_seconds: document.period_seconds,
        debounce_seconds: document.debounce_seconds,
        cooldown_seconds: document.cooldown_seconds,
        max_suppressed: document.max_suppressed,
    })
}

fn disk(document: &DiskDocument) -> Result<DiskLimits, ConfigError> {
    const CEILING: u64 = 1 << 50;
    if document.minimum_free_bytes == 0
        || document.warn_free_bytes < document.minimum_free_bytes
        || document.warn_free_bytes > CEILING
    {
        return Err(ConfigError);
    }
    Ok(DiskLimits {
        minimum_free_bytes: document.minimum_free_bytes,
        warn_free_bytes: document.warn_free_bytes,
    })
}

fn sync(document: &SyncDocument) -> Result<SyncLimits, ConfigError> {
    const OBJECT_CEILING: u64 = 1_000_000;
    const BYTE_CEILING: u64 = 1 << 46;
    if !(1..=OBJECT_CEILING).contains(&document.max_objects)
        || !(1..=BYTE_CEILING).contains(&document.max_bytes)
        || document.warn_objects == 0
        || document.warn_bytes == 0
        || document.warn_objects > document.max_objects
        || document.warn_bytes > document.max_bytes
    {
        return Err(ConfigError);
    }
    Ok(SyncLimits {
        max_objects: document.max_objects,
        max_bytes: document.max_bytes,
        warn_objects: document.warn_objects,
        warn_bytes: document.warn_bytes,
    })
}

fn dispatch(document: &DispatchDocument) -> Result<DispatchLimits, ConfigError> {
    if !(1..=16).contains(&document.max_attempts) || !(1..=1024).contains(&document.dedupe_capacity)
    {
        return Err(ConfigError);
    }
    Ok(DispatchLimits {
        max_attempts: document.max_attempts,
        dedupe_capacity: document.dedupe_capacity,
    })
}
