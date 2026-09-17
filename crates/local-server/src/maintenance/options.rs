//! 封闭的本机维护参数；只接受非秘密根路径和固定请求指纹。

use super::MaintenanceError;
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Inspect,
    CancelPrepared,
    PreserveInstalling,
    CancelRestoreIntent,
    CancelMaintenance,
    AuditFlush,
}

impl Command {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::CancelPrepared => "cancel-prepared",
            Self::PreserveInstalling => "preserve-installing",
            Self::CancelRestoreIntent => "cancel-restore-intent",
            Self::CancelMaintenance => "cancel-maintenance",
            Self::AuditFlush => "audit-flush",
        }
    }

    fn needs_request(self) -> bool {
        matches!(
            self,
            Self::CancelPrepared | Self::PreserveInstalling | Self::CancelRestoreIntent
        )
    }
}

pub(super) struct Options {
    pub(super) command: Command,
    pub(super) config: PathBuf,
    pub(super) vault: PathBuf,
    pub(super) assets: PathBuf,
    pub(super) writer_data: PathBuf,
    pub(super) request: Option<String>,
    pub(super) maintenance: Option<String>,
}

pub(super) fn parse(args: Vec<OsString>) -> Result<Option<Options>, MaintenanceError> {
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    if !matches!(args.len(), 9 | 11) {
        return Err(MaintenanceError::Arguments);
    }
    let command = match args[0].to_str() {
        Some("inspect") => Command::Inspect,
        Some("cancel-prepared") => Command::CancelPrepared,
        Some("preserve-installing") => Command::PreserveInstalling,
        Some("cancel-restore-intent") => Command::CancelRestoreIntent,
        Some("cancel-maintenance") => Command::CancelMaintenance,
        Some("audit-flush") => Command::AuditFlush,
        _ => return Err(MaintenanceError::Arguments),
    };
    let mut config = None;
    let mut vault = None;
    let mut assets = None;
    let mut writer_data = None;
    let mut request = None;
    let mut maintenance = None;
    for pair in args[1..].chunks_exact(2) {
        let target = match pair[0].to_str() {
            Some("--config") => &mut config,
            Some("--vault") => &mut vault,
            Some("--assets") => &mut assets,
            Some("--writer-data") => &mut writer_data,
            Some("--request") if command.needs_request() && request.is_none() => {
                let value = pair[1].to_str().ok_or(MaintenanceError::Arguments)?;
                if value.len() != 64
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(MaintenanceError::Arguments);
                }
                request = Some(value.to_owned());
                continue;
            }
            Some("--maintenance")
                if command == Command::CancelMaintenance && maintenance.is_none() =>
            {
                let value = pair[1].to_str().ok_or(MaintenanceError::Arguments)?;
                if value.len() != 64
                    || !value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(MaintenanceError::Arguments);
                }
                maintenance = Some(value.to_owned());
                continue;
            }
            _ => return Err(MaintenanceError::Arguments),
        };
        if target.is_some() {
            return Err(MaintenanceError::Arguments);
        }
        let value = pair[1].to_str().ok_or(MaintenanceError::Arguments)?;
        if !value.starts_with('/') || value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(MaintenanceError::Arguments);
        }
        *target = Some(PathBuf::from(value));
    }
    if command.needs_request() != request.is_some()
        || (command == Command::CancelMaintenance) != maintenance.is_some()
    {
        return Err(MaintenanceError::Arguments);
    }
    Ok(Some(Options {
        command,
        config: config.ok_or(MaintenanceError::Arguments)?,
        vault: vault.ok_or(MaintenanceError::Arguments)?,
        assets: assets.ok_or(MaintenanceError::Arguments)?,
        writer_data: writer_data.ok_or(MaintenanceError::Arguments)?,
        request,
        maintenance,
    }))
}
