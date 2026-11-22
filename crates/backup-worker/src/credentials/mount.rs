//! 凭据所在的挂载是不是「不会换出到硬盘的只读内存文件系统」。
//!
//! ramfs 从不换出。tmpfs 默认会换出，只有挂载时带 `noswap` 的不会；systemd 在够新的内核上给凭据用的就是
//! 带 `noswap` 的只读 tmpfs（BUG-043，169 的 systemd 259 实测）。`statfs` 看不出 `noswap`，所以拿描述符的
//! 挂载号到 `/proc/self/mountinfo` 找那一行，看超级块选项。判定是纯函数，读取层只有 `noswap` 一个入口。
use rustix::{
    fd::AsFd,
    fs::{self, AtFlags, StatxFlags},
};
use std::{fs::File, io::Read};

/// procfs 的魔数（内核 `PROC_SUPER_MAGIC`）：挂载表必须真的来自 procfs，不认放在同一路径上的普通文件。
const PROC: u64 = 0x9fa0;
/// 挂载表正文最多读多少；169 上整份不到 8 KiB，超过 1 MiB 的不读、算核不出。
const TABLE_LIMIT: u64 = 1024 * 1024;

/// 描述符所在的 tmpfs 挂载是不是只读而且带 `noswap`：取它的挂载号，到本进程的挂载表里找那一行。
/// 内核不给挂载号、挂载表读不到或太长，都算不是。
pub(super) fn noswap(fd: &impl AsFd) -> bool {
    let Ok(seen) = fs::statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID) else {
        return false;
    };
    if seen.stx_mask & StatxFlags::MNT_ID.bits() == 0 {
        return false;
    }
    table().is_some_and(|table| tmpfs_readonly_noswap(&table, seen.stx_mnt_id))
}
fn table() -> Option<String> {
    let file = File::open("/proc/self/mountinfo").ok()?;
    if fs::fstatfs(&file).ok()?.f_type as u64 != PROC {
        return None;
    }
    let mut text = String::new();
    (&file)
        .take(TABLE_LIMIT + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= TABLE_LIMIT).then_some(text)
}

/// mountinfo 正文里挂载号是 `mount_id` 的那一行，是不是 tmpfs、超级块选项里同时有 `ro` 和 `noswap`。
/// 找不到、同一个号出现不止一行、格式不认识，都算不是。
pub(super) fn tmpfs_readonly_noswap(mountinfo: &str, mount_id: u64) -> bool {
    // 每行：挂载号 父号 设备号 根 挂载点 挂载选项 [可选字段…] - 文件系统类型 来源 超级块选项。
    // 路径里的空格在这个文件里写成 \040，所以第一个「 - 」就是分隔符。
    let mut matching = mountinfo.lines().filter_map(|line| {
        let (head, tail) = line.split_once(" - ")?;
        let id = head.split(' ').next()?.parse::<u64>().ok()?;
        (id == mount_id).then_some(tail)
    });
    let (Some(tail), None) = (matching.next(), matching.next()) else {
        return false;
    };
    let mut fields = tail.split(' ');
    let (Some("tmpfs"), Some(_source), Some(options), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return false;
    };
    let has = |wanted: &str| options.split(',').any(|option| option == wanted);
    has("ro") && has("noswap")
}

#[cfg(test)]
#[path = "../../tests/support/credentials_mount.rs"]
mod tests;
