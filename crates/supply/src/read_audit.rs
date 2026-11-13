//! 人侧审计查询投影：只用经段链验证的账本记录，拒绝记录不推断未保存的路径。
use crate::audit::{ACTION_DENIED, CONTENT_READ};
use serde::{Deserialize, Serialize};
use std::path::Path;
use studiolo_audit_ledger::{AuditError, AuditReadView, AuditRecord};

/// 固定两种查询；成功读取和拒绝动作不混成同一类记录。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOutcome {
    /// 已交付内容。
    Read,
    /// 被拒绝的接入方动作。
    Denied,
}

/// HTTP 与查询内核共用的封闭输入；时间必须带时区。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadAuditQuery {
    /// 成功或拒绝类别。
    pub outcome: ReadOutcome,
    /// 可选的AI类型。
    pub agent_type: Option<String>,
    /// 可选的实例号；拒绝记录可为匿名。
    pub instance_id: Option<String>,
    /// 含边界的起始时间。
    pub since: Option<String>,
    /// 含边界的截止时间。
    pub until: Option<String>,
    /// 成功读取路径的字面前缀；不是通配表达式。
    pub path_prefix: Option<String>,
    /// 账本偏移排他上界；空值从最新页开始。
    pub before: Option<u64>,
}
impl ReadAuditQuery {
    /// 输入验证不接触账本，拒绝动作没有路径这一维。
    pub fn validate(&self) -> bool {
        let since = self
            .since
            .as_deref()
            .map(str::parse::<jiff::Timestamp>)
            .transpose();
        let until = self
            .until
            .as_deref()
            .map(str::parse::<jiff::Timestamp>)
            .transpose();
        matches!((&since, &until), (Ok(a), Ok(b)) if a.zip(*b).is_none_or(|(a,b)| a <= b))
            && self
                .agent_type
                .as_ref()
                .is_none_or(|s| crate::validate_type_name(s).is_ok())
            && self
                .instance_id
                .as_ref()
                .is_none_or(|s| s.len() == 26 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
            && self.path_prefix.as_ref().is_none_or(|s| bounded(s, 1024))
            && (self.outcome != ReadOutcome::Denied || self.path_prefix.is_none())
            && self
                .before
                .is_none_or(|n| (1..=9_007_199_254_740_991).contains(&n))
    }
}

/// 固定输出，不暴露原始审计 JSON。
#[derive(Debug, Serialize)]
pub struct ReadAuditRow {
    /// 权威账本偏移。
    pub offset: u64,
    /// 权威账本时间。
    pub recorded_at: String,
    /// 可选的AI类型。
    pub agent_type: Option<String>,
    /// 可选的实例号；拒绝记录可为匿名。
    pub instance_id: Option<String>,
    /// 实际工具或被拒绝的动作。
    pub action: String,
    /// 成功供给路径；拒绝记录恒为空。
    pub path: Option<String>,
    /// 供给时版本，支持内容哈希与策略版本。
    pub revision: Option<String>,
    /// 拒绝原因码；成功记录为空。
    pub reason: Option<String>,
}
/// 一个已验证窗口及继续查询所需的非秘密游标。
#[derive(Debug, Serialize)]
pub struct ReadAuditPage {
    /// 查询快照时间。
    pub generated_at: String,
    /// 成功或拒绝类别。
    pub outcome: ReadOutcome,
    /// 当前账本最高偏移。
    pub head_offset: u64,
    /// 本窗口扫描的权威记录数，不是匹配数。
    pub scanned: usize,
    /// 更早一页的排他上界。
    pub next_before: Option<u64>,
    /// 经验证的匹配记录，新到旧。
    pub records: Vec<ReadAuditRow>,
}

/// 只能投影查询，不持有身份或审计写者能力。
pub struct AgentReadAudit {
    ledger: AuditReadView,
}
impl AgentReadAudit {
    pub(crate) fn open(directory: &Path) -> Result<Self, AuditError> {
        Ok(Self {
            ledger: AuditReadView::open_existing(directory)?,
        })
    }
    /// 在有界权威窗口内应用四维筛选；不返回原始事件正文。
    pub fn query(&self, query: &ReadAuditQuery) -> Result<ReadAuditPage, AuditError> {
        if !query.validate() {
            return Err(AuditError::Serialization);
        }
        let since = query
            .since
            .as_deref()
            .map(str::parse::<jiff::Timestamp>)
            .transpose()
            .map_err(|_| AuditError::Serialization)?;
        let until = query
            .until
            .as_deref()
            .map(str::parse::<jiff::Timestamp>)
            .transpose()
            .map_err(|_| AuditError::Serialization)?;
        let page = self.ledger.scan_page(query.before, |record| {
            if since.is_some_and(|t| record.recorded_at() < t)
                || until.is_some_and(|t| record.recorded_at() > t)
            {
                return Ok(None);
            }
            let expected = match query.outcome {
                ReadOutcome::Read => CONTENT_READ,
                ReadOutcome::Denied => ACTION_DENIED,
            };
            if record.event().action() != expected {
                return Ok(None);
            }
            let row = project(record, query.outcome)?;
            if query
                .agent_type
                .as_ref()
                .is_some_and(|s| row.agent_type.as_ref() != Some(s))
                || query
                    .instance_id
                    .as_ref()
                    .is_some_and(|s| row.instance_id.as_ref() != Some(s))
                || query
                    .path_prefix
                    .as_ref()
                    .is_some_and(|s| row.path.as_ref().is_none_or(|p| !p.starts_with(s)))
            {
                return Ok(None);
            }
            Ok(Some(row))
        })?;
        // 浏览器以 head+1 钉住第一页的排他上界，往返翻页不受后续追加挤动。
        if page.head_offset >= 9_007_199_254_740_991 {
            return Err(AuditError::CorruptStore);
        }
        Ok(ReadAuditPage {
            generated_at: jiff::Timestamp::now().to_string(),
            outcome: query.outcome,
            head_offset: page.head_offset,
            scanned: page.scanned,
            next_before: page.next_before,
            records: page.records,
        })
    }
}
fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn field(value: &serde_json::Value, key: &str, max: usize) -> Result<String, AuditError> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|s| bounded(s, max))
        .map(str::to_owned)
        .ok_or(AuditError::CorruptStore)
}
fn optional(
    value: &serde_json::Value,
    key: &str,
    max: usize,
) -> Result<Option<String>, AuditError> {
    if value.get(key).is_some_and(serde_json::Value::is_null) {
        Ok(None)
    } else {
        field(value, key, max).map(Some)
    }
}
fn project(record: &AuditRecord, outcome: ReadOutcome) -> Result<ReadAuditRow, AuditError> {
    let body = record.event().payload().as_value();
    let agent_type = optional(body, "agent_type", 32)?;
    let instance_id = optional(body, "agent_instance", 64)?;
    let (action, path, revision, reason) = match outcome {
        ReadOutcome::Read => (
            field(body, "tool", 128)?,
            Some(field(body, "path", 4096)?),
            Some(field(body, "revision", 128)?),
            None,
        ),
        ReadOutcome::Denied => (
            field(body, "action", 128)?,
            None,
            None,
            Some(field(body, "reason", 128)?),
        ),
    };
    if record.offset() > 9_007_199_254_740_991 {
        return Err(AuditError::CorruptStore);
    }
    Ok(ReadAuditRow {
        offset: record.offset(),
        recorded_at: record.recorded_at().to_string(),
        agent_type,
        instance_id,
        action,
        path,
        revision,
        reason,
    })
}
