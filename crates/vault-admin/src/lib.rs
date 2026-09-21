//! 从管理员冻结的私有快照首建 canonical；不提供网络身份或持续同步能力。
//!
//! `plan` 无持久写入。两个有界扫描相同并不构成跨文件原子快照：调用者必须在整个
//! plan/initialize 窗口保持来源冻结和父目录可信。初始化部分失败后保留全部现场，
//! 不自动清理、不修复、不续跑；普通服务仍只使用 Broker 的 `open_existing`。

mod options;
mod source;

pub use options::{Command, Options, parse};
use serde::Serialize;
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload, SqliteAuditLedger,
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{FsVaultBroker, Mutation, VaultReader, VaultWriter};
use subtle::ConstantTimeEq;

/// 固定脱敏错误；不携带路径、源正文、凭据或底层错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// 参数或摘要格式不符合封闭命令协议。
    #[error("参数无效；请按 --help 提供明确的非秘密路径、范围和计划摘要")]
    Arguments,
    /// 根目录、配置或分离关系不满足要求。
    #[error("批准根或私有配置无效；禁止重叠、链接、非空目标或自动修复")]
    Paths,
    /// 私有快照扫描失败或包含不支持的输入。
    #[error("冻结来源不完整或不安全；拒绝诊断、所选附件、受保护路径及超限输入")]
    Source,
    /// 已批准对象在检查中发生变化。
    #[error("批准对象或来源发生变化；未确认当前计划，不允许继续")]
    Changed,
    /// 提交给初始化的摘要与实际计划不一致。
    #[error("计划摘要不匹配；未初始化，请重新审核冻结来源和目标")]
    PlanMismatch,
    /// 既有权威审计不可用。
    #[error("既有审计库不可用；不创建审计库、不继续初始化")]
    Audit,
    /// Broker 已拒绝操作，现场可能已经部分写入。
    #[error("canonical 首建或导入失败；可能已有部分写入，保留现场，禁止自动重试")]
    Broker,
    /// 副作用之后审计不能确认。
    #[error("首建操作已尝试但结果审计未确认；保留现场并核对审计，禁止自动重试")]
    AuditPending,
}

/// 可公开显示的计划摘要；不包含源正文和逐文件列表。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlanSummary {
    /// 固定计划协议版本。
    pub version: u8,
    /// 绑定配置、范围、根身份和排序文件内容的 SHA-256。
    pub plan_digest: String,
    /// 将导入的 Markdown 文件数。
    pub files: usize,
    /// 将导入的总内容字节数。
    pub bytes: u64,
    /// 操作者明确指定并规范排序的逻辑范围。
    pub allow: Vec<VaultPath>,
}

/// 完成结果；只有全部导入且结果审计持久化才返回。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Initialized {
    /// 新 canonical Vault 的稳定 UUID。
    pub vault_uuid: String,
    /// 成功导入的 Markdown 文件数。
    pub files: usize,
    /// 成功导入的内容字节数。
    pub bytes: u64,
    /// 已批准并执行的计划摘要。
    pub plan_digest: String,
}

/// 读取既有批准根和冻结快照，不初始化存储、不打开审计写连接。
pub fn plan(options: &Options) -> Result<PlanSummary, Error> {
    Ok(source::Prepared::capture(options)?.summary)
}

/// 重算精确计划后记录本地 OS 意图，只对既有空目录执行一次性首建。
///
/// 来源必须由操作者冻结。结果失败时可能留下 Broker 标记和已提交文档；绝不删除现场。
pub fn initialize(options: &Options, expected_plan: &str) -> Result<Initialized, Error> {
    let expected = decode_digest(expected_plan)?;
    let prepared = source::Prepared::capture(options)?;
    let actual = decode_digest(&prepared.summary.plan_digest)?;
    if !bool::from(expected.ct_eq(&actual)) {
        return Err(Error::PlanMismatch);
    }
    prepared.revalidate()?;
    let audit = SqliteAuditLedger::open_existing(prepared.config.audit_directory())
        .map_err(|_| Error::Audit)?;
    prepared.revalidate()?;
    let id = AuditEventId::parse(format!("bootstrap-{}", prepared.summary.plan_digest))
        .map_err(|_| Error::Audit)?;
    append(
        &audit,
        &id,
        AuditPhase::Intent,
        &prepared.summary,
        "planned",
        0,
    )?;
    let mut imported = 0;
    let outcome = import(&prepared, &mut imported);
    append(
        &audit,
        &id,
        AuditPhase::Outcome,
        &prepared.summary,
        if outcome.is_ok() {
            "completed"
        } else {
            "failed_preserved"
        },
        imported,
    )
    .map_err(|_| Error::AuditPending)?;
    outcome
}

fn import(prepared: &source::Prepared, imported: &mut usize) -> Result<Initialized, Error> {
    prepared.revalidate()?;
    prepared.empty_targets()?;
    let mut broker =
        FsVaultBroker::initialize_empty(&prepared.options.vault, &prepared.options.writer_data)
            .map_err(|_| Error::Broker)?;
    prepared.revalidate()?;
    for (index, note) in prepared.notes.iter().enumerate() {
        prepared.revalidate()?;
        broker
            .mutate(Mutation::create(
                note.path.clone(),
                note.content.clone(),
                format!("bootstrap-{}-{index}", prepared.summary.plan_digest),
            ))
            .map_err(|_| Error::Broker)?;
        *imported += 1;
    }
    prepared.revalidate()?;
    Ok(Initialized {
        vault_uuid: broker.vault_uuid().to_string(),
        files: *imported,
        bytes: prepared.summary.bytes,
        plan_digest: prepared.summary.plan_digest.clone(),
    })
}

fn append(
    audit: &SqliteAuditLedger,
    id: &AuditEventId,
    phase: AuditPhase,
    summary: &PlanSummary,
    status: &str,
    imported: usize,
) -> Result<(), Error> {
    let payload = SanitizedPayload::try_from(serde_json::json!({
        "actor_kind": "local_os_bootstrap",
        "os_uid": rustix::process::getuid().as_raw(),
        "authority": "explicit_offline_plan",
        "plan_digest": summary.plan_digest,
        "files": summary.files,
        "bytes": summary.bytes,
        "allow": summary.allow,
        "status": status,
        "imported_files": imported,
    }))
    .map_err(|_| Error::Audit)?;
    audit
        .append(AuditEvent::new(
            id.clone(),
            phase,
            "vault.bootstrap",
            payload,
        ))
        .map(|_| ())
        .map_err(|_| Error::Audit)
}

fn decode_digest(value: &str) -> Result<[u8; 32], Error> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Arguments);
    }
    let mut digest = [0; 32];
    for (index, slot) in digest.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::Arguments)?;
    }
    Ok(digest)
}
