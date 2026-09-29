//! 残留删除的审计日记：先记事件身份，再记删除可能开始；结果落盘后才追加 Outcome。
use sha2::{Digest, Sha256};

/// 崩溃期间的删除结果可能未知；未知不得伪造为没有删除。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeResult {
    /// 实删文件数；删除期间中断时未知。
    pub files: Option<u64>,
    /// 实删目录数；删除期间中断时未知。
    pub directories: Option<u64>,
    /// 是否整趟删除成功；删除期间中断时未知。
    pub applied: Option<bool>,
}

/// 至多一个未完成审计的清除操作，绑定原事件、文件夹与确认时的计划计数。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingPurge {
    /// 随机事件标识的小写十六进制，重开不得换号。
    pub event_id: String,
    /// 已获确认的排除文件夹。
    pub folder: String,
    /// 确认前核验的文件数。
    pub planned_files: u64,
    /// 确认前核验的目录数（含顶层）。
    pub planned_directories: u64,
    /// 已持久许可删除，崩溃后无法由文件现状判断实际删了多少。
    pub deletion_started: bool,
    /// 删除返回后先持久的结果；用于原样补记 Outcome。
    pub result: Option<PurgeResult>,
}

impl PendingPurge {
    pub(super) fn valid(&self) -> bool {
        let hex = self.event_id.len() == 64
            && self
                .event_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let counts = self.planned_files.saturating_add(self.planned_directories) <= 65_569;
        let result = self.result.as_ref().is_none_or(|result| {
            match (result.files, result.directories, result.applied) {
                (Some(files), Some(directories), Some(applied)) => {
                    files.saturating_add(directories) <= 65_569
                        && (!applied || self.deletion_started)
                        && (self.deletion_started || (files == 0 && directories == 0))
                }
                (None, None, None) => self.deletion_started,
                _ => false,
            }
        });
        hex && counts
            && result
            && studiolo_host_config::sync_transport::excluded_folder_acceptable(&self.folder)
    }

    pub(super) fn checksum(&self, digest: &mut Sha256) {
        for field in [&self.event_id, &self.folder] {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field.as_bytes());
        }
        digest.update(self.planned_files.to_be_bytes());
        digest.update(self.planned_directories.to_be_bytes());
        digest.update([
            u8::from(self.deletion_started),
            u8::from(self.result.is_some()),
        ]);
        if let Some(result) = &self.result {
            for count in [result.files, result.directories] {
                digest.update([u8::from(count.is_some())]);
                digest.update(count.unwrap_or_default().to_be_bytes());
            }
            digest.update([
                u8::from(result.applied.is_some()),
                u8::from(result.applied.unwrap_or_default()),
            ]);
        }
    }
}
