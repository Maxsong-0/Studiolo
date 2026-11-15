//! 固定队列目录里的作业键。文件名必须等于 checkpoint 标识，正文不能夹带路径或秘密。
use crate::{ProcessError, Result};
use std::{fs, path::Path};
use studiolo_backup::worker::state::JobKey;

/// 读取队列。目录不存在表示没有作业，不创建目录。任一文件不合法则整批拒绝。
pub fn read_queue(directory: &Path) -> Result<Vec<JobKey>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(ProcessError::Configuration),
    };
    let mut keys = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| ProcessError::Configuration)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(ProcessError::Configuration);
        };
        if name.starts_with('.') {
            continue;
        }
        let Some(checkpoint) = name.strip_suffix(".json") else {
            return Err(ProcessError::Configuration);
        };
        if checkpoint.contains('/') || checkpoint.contains("..") || checkpoint.is_empty() {
            return Err(ProcessError::Configuration);
        }
        let bytes = fs::read(entry.path()).map_err(|_| ProcessError::Configuration)?;
        if bytes.len() > 4096 {
            return Err(ProcessError::Configuration);
        }
        let key: JobKey =
            serde_json::from_slice(&bytes).map_err(|_| ProcessError::Configuration)?;
        if key.checkpoint_id.as_str() != checkpoint {
            return Err(ProcessError::Configuration);
        }
        keys.push(key);
    }
    keys.sort_by_key(|key| key.sequence);
    Ok(keys)
}
