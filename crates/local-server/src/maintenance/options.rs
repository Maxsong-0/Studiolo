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
    /// 显式初始化双向 Bridge 平面的四项私有存储；任一已存在即拒绝。
    SyncInit,
    /// 显式准备已批准的 canonical 目录锚点，不改任何文件或传输状态。
    SyncAnchors,
    /// 静止窗口内显式重绑定传输安全配置，保留既有安装身份与序号。
    SyncRebind,
    /// 清除某个已不再排除的文件夹在镜像视图里的残留；须先停 `studiolo-human`。
    SyncPurgeResidue,
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
            Self::SyncInit => "sync-init",
            Self::SyncAnchors => "sync-anchors",
            Self::SyncRebind => "sync-rebind",
            Self::SyncPurgeResidue => "sync-purge-residue",
        }
    }

    fn needs_request(self) -> bool {
        matches!(
            self,
            Self::CancelPrepared | Self::PreserveInstalling | Self::CancelRestoreIntent
        )
    }

    fn needs_transport(self) -> bool {
        matches!(
            self,
            Self::SyncInit | Self::SyncAnchors | Self::SyncRebind | Self::SyncPurgeResidue
        )
    }

    /// 每个命令固定的实参个数：命令名、四个根参数对，再加各自专属的参数对；多一个少一个都拒绝。
    fn arity(self) -> usize {
        match self {
            Self::Inspect | Self::AuditFlush => 9,
            Self::CancelPrepared
            | Self::PreserveInstalling
            | Self::CancelRestoreIntent
            | Self::CancelMaintenance
            | Self::SyncInit
            | Self::SyncAnchors
            | Self::SyncRebind => 11,
            Self::SyncPurgeResidue => 13,
        }
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
    /// 双向 Bridge 审批文档（`sync-init`／`sync-anchors`／`sync-rebind`／`sync-purge-residue`）。
    pub(super) sync_transport: Option<PathBuf>,
    /// 要清除镜像残留的排除文件夹名（`sync-purge-residue`）。
    pub(super) folder: Option<String>,
}

pub(super) fn parse(args: Vec<OsString>) -> Result<Option<Options>, MaintenanceError> {
    if args.len() == 1 && args[0] == "--help" {
        return Ok(None);
    }
    let command = match args.first().and_then(|value| value.to_str()) {
        Some("inspect") => Command::Inspect,
        Some("cancel-prepared") => Command::CancelPrepared,
        Some("preserve-installing") => Command::PreserveInstalling,
        Some("cancel-restore-intent") => Command::CancelRestoreIntent,
        Some("cancel-maintenance") => Command::CancelMaintenance,
        Some("audit-flush") => Command::AuditFlush,
        Some("sync-init") => Command::SyncInit,
        Some("sync-anchors") => Command::SyncAnchors,
        Some("sync-rebind") => Command::SyncRebind,
        Some("sync-purge-residue") => Command::SyncPurgeResidue,
        _ => return Err(MaintenanceError::Arguments),
    };
    if args.len() != command.arity() {
        return Err(MaintenanceError::Arguments);
    }
    let mut config = None;
    let mut vault = None;
    let mut assets = None;
    let mut writer_data = None;
    let mut request = None;
    let mut maintenance = None;
    let mut sync_transport = None;
    let mut folder = None;
    for pair in args[1..].chunks_exact(2) {
        let target = match pair[0].to_str() {
            Some("--config") => &mut config,
            Some("--vault") => &mut vault,
            Some("--assets") => &mut assets,
            Some("--writer-data") => &mut writer_data,
            Some("--sync-transport") if command.needs_transport() && sync_transport.is_none() => {
                &mut sync_transport
            }
            Some("--folder") if command == Command::SyncPurgeResidue && folder.is_none() => {
                let value = pair[1].to_str().ok_or(MaintenanceError::Arguments)?;
                // 与审批文档 `excluded_folders` 同一条规则；名字不合法就不去碰任何目录。
                if !studiolo_host_config::sync_transport::excluded_folder_acceptable(value) {
                    return Err(MaintenanceError::Arguments);
                }
                folder = Some(value.to_owned());
                continue;
            }
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
        || command.needs_transport() != sync_transport.is_some()
        || (command == Command::SyncPurgeResidue) != folder.is_some()
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
        sync_transport,
        folder,
    }))
}
