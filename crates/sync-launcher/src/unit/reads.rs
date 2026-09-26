//! cgroupfs／proc／session 文件的只读事实（守卫按精确记号登记）：cgroup 事实只从
//! `/sys/fs/cgroup<组>` 读，进程事实只从 `/proc` 读，所有读取都有上限。

use crate::facts;
use crate::protocol::{LimitsFrame, Verdict};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use studiolo_sync_transport::linux::cgroup;

/// 文本文件读取上限（mountinfo／passwd／group／cgroup 文件都远小于它）。
const TEXT_CAP: usize = 4 * 1024 * 1024;

/// `ControlGroup` 属性对应的 cgroupfs 目录。
#[must_use]
pub fn cgroup_dir(control_group: &str) -> PathBuf {
    PathBuf::from(format!("{}{control_group}", facts::CGROUP_ROOT))
}

/// 读 `cgroup.events`；目录已被回收返回 `Ok(None)`（内核只允许删除空组，调用方再核对 unit 已等到主进程）。
///
/// # Errors
/// 文件格式不对返回 `Protocol`；其它读取失败返回 `ManagerUnavailable`。
pub fn events(control_group: &str) -> Result<Option<(bool, bool)>, Verdict> {
    match read_text(&cgroup_dir(control_group).join("cgroup.events")) {
        Ok(text) => {
            let events = cgroup::parse_events(&text).map_err(|_| Verdict::Protocol)?;
            Ok(Some((events.populated, events.frozen)))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Verdict::ManagerUnavailable),
    }
}

/// 实测限额：`memory.max`／`memory.swap.max`／`pids.max`／`cpu.max` 各读一次。
///
/// # Errors
/// 读不到返回 `ManagerUnavailable`；格式不对返回 `Protocol`。
pub fn limits(control_group: &str) -> Result<LimitsFrame, Verdict> {
    let dir = cgroup_dir(control_group);
    let read = |name: &str| read_text(&dir.join(name)).map_err(|_| Verdict::ManagerUnavailable);
    facts::limits(
        &read("memory.max")?,
        &read("memory.swap.max")?,
        &read("pids.max")?,
        &read("cpu.max")?,
    )
}

/// `MainPID` 当前执行的程序（`/proc/<pid>/exe`）。
#[must_use]
pub fn main_exe(pid: u64) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// 跟随链接后的 `(dev, ino, len)`；不存在返回 `None`。
#[must_use]
pub fn stat(path: &Path) -> Option<(u64, u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino(), metadata.len()))
}

/// 是否有 cgroup v2（`cgroup.controllers` 在挂载点根上）。
#[must_use]
pub fn cgroup_v2() -> bool {
    stat(&Path::new(facts::CGROUP_ROOT).join("cgroup.controllers")).is_some()
}

/// 读有界文本文件。
///
/// # Errors
/// 读不到或超过上限返回 IO 错误。
pub fn read_text(path: &Path) -> io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    if text.len() > TEXT_CAP {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(text)
}

/// 读不超过 `cap` 字节的文件；更大即拒绝（不截断后当作完整正文）。
///
/// # Errors
/// 不存在、读不到或超过 `cap` 返回 IO 错误。
pub fn read_bounded(path: &Path, cap: usize) -> io::Result<Vec<u8>> {
    let (_, _, len) = stat(path).ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    if len > cap as u64 {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let bytes = std::fs::read(path)?;
    if bytes.len() > cap {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(bytes)
}
