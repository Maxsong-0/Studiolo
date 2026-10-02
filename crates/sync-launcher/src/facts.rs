//! launcher 的纯事实解析与判定（无 IO）：unit 名、systemd 输出解析、mountinfo／passwd 解析、
//! 限额解析。七项隔离在 `facts/isolation.rs`，launcher↔垫片的两种帧在 `facts/frame.rs`。
//! 真实读取在 `unit.rs`，编排在 `handler.rs`。

mod frame;
mod isolation;

pub use frame::{
    LaunchFacts, LaunchFactsKind, decode_launch_facts, encode_launch_facts, split_report,
};
pub use isolation::{IsolationInputs, isolation};

use crate::protocol::{LaunchFrame, LimitsFrame, Verdict};
use std::collections::BTreeMap;
use std::path::Path;
use studiolo_sync_transport::linux::cgroup;
/// 与 owner 共用的逐次载荷单元命名规则。
pub use studiolo_sync_transport::linux::service::payload_unit_name as unit_name;
use studiolo_sync_transport::owned_run::RunOperation;

/// `PrivatePIDs=` 自 systemd 257 起可用（systemd.exec(5)「Added in version 257」）；更低版本建不了 PID 视图。
pub const MIN_SYSTEMD_VERSION: u32 = 257;
/// cgroup v2 挂载点；`ControlGroup` 属性拼在它后面就是绝对路径。
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// 16 字节的小写十六进制（session 目录名、InvocationID 比对用）。
#[must_use]
pub fn hex16(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 解析 32 位十六进制；允许 UUID 的连字符（`/proc/sys/kernel/random/boot_id`）。
#[must_use]
pub fn parse_hex16(text: &str) -> Option<[u8; 16]> {
    let compact: String = text.trim().chars().filter(|c| *c != '-').collect();
    if compact.len() != 32 || !compact.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 16];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&compact[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// `systemctl --version` 首行 `systemd 259 (259.5-0ubuntu3.4)` 的主版本。
#[must_use]
pub fn systemd_version(text: &str) -> Option<u32> {
    let mut words = text.lines().next()?.split_whitespace();
    if words.next()? != "systemd" {
        return None;
    }
    words.next()?.parse().ok()
}

/// `/proc/self/mountinfo` 里挂在 `mount_point` 上的最上层（最后一条）挂载的 `maj:min`。
#[must_use]
pub fn mount_device(mountinfo: &str, mount_point: &Path) -> Option<String> {
    let wanted = mount_point.to_str()?;
    mountinfo
        .lines()
        .filter_map(|line| {
            // 字段：0 id、1 parent、2 maj:min、3 root、4 挂载点（空格等以 `\ooo` 转义）。
            let mut fields = line.split(' ');
            let device = fields.nth(2)?;
            let point = fields.nth(1)?;
            (unescape(point) == wanted).then(|| device.to_owned())
        })
        .next_back()
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        // `\ooo`：反斜杠后恰三位八进制（mountinfo 对空格、制表、换行、反斜杠的转义）。
        if bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && let Ok(value) = u8::from_str_radix(&text[index + 1..index + 4], 8)
        {
            out.push(value);
            index += 4;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `/etc/passwd` 里 uid 恰为 `uid` 且主组为 `gid` 的用户名。
#[must_use]
pub fn passwd_name(passwd: &str, uid: u32, gid: u32) -> Option<String> {
    passwd.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        let matches = fields.len() >= 4
            && fields[2].parse::<u32>().ok() == Some(uid)
            && fields[3].parse::<u32>().ok() == Some(gid);
        matches.then(|| fields[0].to_owned())
    })
}

/// `/etc/group` 里 gid 恰为 `gid` 的组名。
#[must_use]
pub fn group_name(group: &str, gid: u32) -> Option<String> {
    group.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        (fields.len() >= 3 && fields[2].parse::<u32>().ok() == Some(gid))
            .then(|| fields[0].to_owned())
    })
}

/// 专用身份：用户存在、主组相符、组存在，且不是任何其它组的成员（补充组为空）。
#[must_use]
pub fn dedicated_identity(passwd: &str, group: &str, uid: u32, gid: u32) -> bool {
    let Some(name) = passwd_name(passwd, uid, gid) else {
        return false;
    };
    if group_name(group, gid).is_none() {
        return false;
    }
    !group.lines().any(|line| {
        let fields: Vec<&str> = line.split(':').collect();
        fields.len() >= 4
            && fields[2].parse::<u32>().ok() != Some(gid)
            && fields[3].split(',').any(|member| member == name)
    })
}

/// slice 名在 cgroup 树上的展开：连字符表示层级，`a-b.slice` 住在 `a.slice/a-b.slice`。
#[must_use]
pub fn slice_cgroup_path(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".slice")?;
    if stem.is_empty() || stem.starts_with('-') || stem.ends_with('-') || stem.contains("--") {
        return None;
    }
    let mut levels = Vec::new();
    let mut prefix = String::new();
    for part in stem.split('-') {
        if !prefix.is_empty() {
            prefix.push('-');
        }
        prefix.push_str(part);
        levels.push(format!("{prefix}.slice"));
    }
    Some(levels.join("/"))
}

/// 批准前缀必须是「manager 根 + slice 的完整层级展开」：系统 manager 的根是 `/sys/fs/cgroup`，
/// 用户 manager 的根以 `.service` 结尾（`…/user@<uid>.service`）。返回 transient unit 该用的 `--slice=` 名。
#[must_use]
pub fn slice_from_prefix(prefix: &Path) -> Option<String> {
    let text = prefix.to_str()?;
    let name = prefix.file_name()?.to_str()?;
    let expansion = format!("/{}", slice_cgroup_path(name)?);
    let root = text.strip_suffix(expansion.as_str())?;
    (root == CGROUP_ROOT || (root.starts_with(CGROUP_ROOT) && root.ends_with(".service")))
        .then(|| name.to_owned())
}

/// 实测 `ControlGroup` 拼上挂载点后必须落在批准前缀之内。
#[must_use]
pub fn within_prefix(control_group: &str, prefix: &Path) -> bool {
    prefix.to_str().is_some_and(|prefix| {
        cgroup::within_prefix(&format!("{CGROUP_ROOT}{control_group}"), prefix)
    })
}

/// 进 systemd 属性值的路径不能带空白或控制字符（属性值按空格分列，不做引号转义）。
#[must_use]
pub fn property_path_acceptable(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|text| !text.chars().any(|c| c.is_whitespace() || c.is_control()))
}

/// `LaunchFrame` 的固定操作名与模式拼成 `RunOperation`；名字不认或模式不配即协议错误。
///
/// # Errors
/// 操作名不在闭集内、`set_mode` 缺模式或其它操作带模式时返回 `Protocol`。
pub fn operation(frame: &LaunchFrame) -> Result<RunOperation, Verdict> {
    match (frame.operation.as_str(), frame.mode) {
        ("inspect_config", None) => Ok(RunOperation::InspectConfig),
        ("run_once", None) => Ok(RunOperation::RunOnce),
        ("set_mode", Some(mode)) => Ok(RunOperation::SetMode(mode)),
        _ => Err(Verdict::Protocol),
    }
}

/// `systemctl show` 的 `Key=Value` 行；不认识的行丢弃，重复键取最后一次。
#[must_use]
pub fn parse_show(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// 属性值；缺席即空串。
#[must_use]
pub fn value<'a>(show: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    show.get(key).map_or("", String::as_str)
}

/// 数值属性；缺席或不是数字即 `None`。
#[must_use]
pub fn number<T: std::str::FromStr>(show: &BTreeMap<String, String>, key: &str) -> Option<T> {
    value(show, key).trim().parse().ok()
}

/// `ExecMainCode`／`ExecMainStatus`：1＝正常退出（码在 Status），2／3＝信号终止（信号号在 Status），
/// 0＝尚未等到；`Result=success` 不是判据（被配置的终止信号结束也算 success）。
#[must_use]
pub fn reap(show: &BTreeMap<String, String>) -> (bool, Option<i32>, Option<i32>) {
    match (
        number::<i32>(show, "ExecMainCode"),
        number::<i32>(show, "ExecMainStatus"),
    ) {
        (Some(1), Some(status)) => (true, Some(status), None),
        (Some(2 | 3), Some(status)) => (true, None, Some(status)),
        _ => (false, None, None),
    }
}

/// 四个 cgroup 文件的正文 → 线上限额；任一格式不对就是协议错误。
///
/// # Errors
/// 非「数值或 max」形状返回 `Protocol`。
pub fn limits(
    memory_max: &str,
    memory_swap_max: &str,
    pids_max: &str,
    cpu_max: &str,
) -> Result<LimitsFrame, Verdict> {
    Ok(LimitsFrame {
        memory_max: cgroup::parse_scalar(memory_max).map_err(|_| Verdict::Protocol)?,
        memory_swap_max: cgroup::parse_scalar(memory_swap_max).map_err(|_| Verdict::Protocol)?,
        pids_max: cgroup::parse_scalar(pids_max).map_err(|_| Verdict::Protocol)?,
        cpu_max: cgroup::parse_cpu_max(cpu_max).map_err(|_| Verdict::Protocol)?,
    })
}
