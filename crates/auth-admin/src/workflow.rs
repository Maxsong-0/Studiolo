//! 三个封闭本地命令共用既有认证状态机；终端输入是私有可替换Seam。

#[cfg(test)]
#[path = "../tests/support/workflow_tests.rs"]
mod tests;

use crate::{AdminError, options::Command, terminal::TerminalError};
use std::sync::Arc;
use studiolo_audit_ledger::{AuditError, SqliteAuditLedger};
use studiolo_auth::{
    OsSessionRandom, SessionError, SessionManager, SystemSessionClock, Totp, TotpCode,
    TotpEnrollmentState, TotpSeed,
};
use studiolo_host_config::Config;

pub(crate) fn execute(
    command: Command,
    config: Config,
    seed: Option<TotpSeed>,
    read_code: impl FnOnce() -> Result<TotpCode, TerminalError>,
) -> Result<&'static str, AdminError> {
    match command {
        Command::Migrate(1..=3) => {}
        Command::Migrate(_) => return Err(AdminError::Arguments),
        _ if seed.is_none() => return Err(AdminError::Environment),
        _ => {}
    }
    let audit = match command {
        Command::Migrate(_) => SqliteAuditLedger::open(config.audit_directory()),
        _ => SqliteAuditLedger::open_existing(config.audit_directory()),
    }
    .map_err(|error| match error {
        AuditError::MissingStore => AdminError::MissingAudit,
        _ => AdminError::Audit,
    })?;
    let audit = Arc::new(audit);
    let session_config = config.sessions()?;
    let clock = Arc::new(SystemSessionClock);
    let random = Arc::new(OsSessionRandom);
    let sessions = match command {
        Command::Migrate(1) => SessionManager::migrate_v1(
            config.auth_directory(),
            session_config,
            audit,
            clock,
            random,
        ),
        Command::Migrate(2) => SessionManager::migrate_v2(
            config.auth_directory(),
            session_config,
            audit,
            clock,
            random,
        ),
        Command::Migrate(3) => SessionManager::migrate_v3(
            config.auth_directory(),
            session_config,
            audit,
            clock,
            random,
        ),
        Command::Migrate(_) => return Err(AdminError::Arguments),
        _ => SessionManager::open_existing(
            config.auth_directory(),
            session_config,
            audit,
            clock,
            random,
        ),
    }?;
    if matches!(command, Command::Migrate(_)) {
        return Ok(
            "本次显式初始化／迁移完成；本次未执行因子登记，未签发会话或L3。已有旧版须按原步骤逐次迁移。",
        );
    }
    let sessions = Arc::new(sessions);
    let (totp, enrollment) = Totp::attach(sessions.clone(), seed.ok_or(AdminError::Environment)?)?;
    match command {
        Command::AuditFlush => {
            totp.flush_audit()?;
            sessions.flush_audit(256)?;
            Ok(
                "本批原事件补记完成（最多256条会话事件）；补记不激活登记。未失效的待定操作仍须等待，可能还有后续批次。",
            )
        }
        Command::Enroll => {
            match enrollment.state()? {
                TotpEnrollmentState::Enrolled => return Err(SessionError::AlreadyEnrolled.into()),
                TotpEnrollmentState::Pending => return Err(SessionError::OperationPending.into()),
                TotpEnrollmentState::Ready => {}
            }
            enrollment.enroll(read_code()?)?;
            Ok("首次TOTP登记已提交；未签发会话或L3。请按后续受控登录流程继续。")
        }
        Command::Migrate(_) => Err(AdminError::Arguments),
    }
}
