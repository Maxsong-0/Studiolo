//! 不创建/迁移数据库，不登记/替换因子；只装配已批准的离线复制布局。

use crate::{AdminError, options::RestoreOptions, restore_guard::Guard};
use std::{
    io::{self, Read, Write},
    path::Path,
    sync::Arc,
};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_auth::{
    OsSessionRandom, Recovery, RestoreInvalidation, SessionManager, SystemSessionClock, Totp,
    TotpSeed,
};
use studiolo_host_config::Config;
use zeroize::Zeroizing;

const REFERENCE: &str = "studiolo-human-totp-v2";

pub(crate) fn execute(config_path: &Path, options: &RestoreOptions) -> Result<(), AdminError> {
    let guard = Guard::pin(&options.root, config_path)?;
    let config = Config::load(config_path)?;
    if config.auth_directory().as_os_str() != options.root.join("auth").as_os_str()
        || config.audit_directory().as_os_str() != options.root.join("audit").as_os_str()
        || config.seed_ref() != REFERENCE
    {
        return Err(AdminError::RestoreGate);
    }
    guard.revalidate()?;
    let seed = read_seed(io::stdin().lock())?;
    guard.revalidate()?;
    let audit = Arc::new(
        SqliteAuditLedger::open_existing(config.audit_directory())
            .map_err(|_| AdminError::Audit)?,
    );
    let sessions = Arc::new(SessionManager::open_existing_for_restore(
        config.auth_directory(),
        config.sessions()?,
        audit,
        Arc::new(SystemSessionClock),
        Arc::new(OsSessionRandom),
    )?);
    // 登记证明由恢复API直接验证；state()会采样运行时钟并推进到期状态。
    let (_totp, _enrollment) = Totp::attach(sessions.clone(), seed)?;
    let _recovery = Recovery::attach(sessions.clone())?;
    guard.revalidate()?;
    let report = sessions.invalidate_restored_credentials(&options.id)?;
    guard.revalidate()?;
    write_report(&report, io::stdout().lock())
}

fn read_seed(mut input: impl Read) -> Result<TotpSeed, AdminError> {
    let mut bytes = Zeroizing::new([0_u8; 41]);
    let mut length = 0;
    while length < bytes.len() {
        match input.read(&mut bytes[length..]) {
            Ok(0) => break,
            Ok(count) => length += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(AdminError::RestoreInput),
        }
    }
    if length != 40
        || !bytes[..40]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(AdminError::RestoreInput);
    }
    TotpSeed::from_hex(REFERENCE.into(), &bytes[..40]).map_err(|_| AdminError::RestoreInput)
}

fn write_report(report: &RestoreInvalidation, mut output: impl Write) -> Result<(), AdminError> {
    writeln!(output, "{{\"epoch\":{},\"revoked_sessions\":{},\"cleared_l3\":{},\"invalidated_recovery_codes\":{},\"audit_confirmed\":{}}}",
        report.epoch, report.revoked_sessions, report.cleared_l3, report.invalidated_recovery_codes, report.audit_confirmed)
        .and_then(|()| output.flush()).map_err(|_| AdminError::Output)?;
    if !report.audit_confirmed {
        return Err(AdminError::RestoreAuditPending);
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/support/offline_restore_tests.rs"]
mod tests;
