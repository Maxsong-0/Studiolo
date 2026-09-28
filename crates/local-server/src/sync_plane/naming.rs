//! 冲突副本与保全路径的命名（P2 结论 7、ADR-025 D5／D6）；纯函数，无 IO、无时钟。
//!
//! headless 生成的副本形如 `<名> (Conflicted copy <设备名> <UTC yyyyMMddHHmm>).<ext>`；
//! Studiolo 自己生成的保全副本用固定设备名 `studiolo-canonical`，
//! 放在范围外的 `ops/bridge-conflicts/`（冲突）与 `ops/bridge-rollback/`（D5 出站前保全）。

use sha2::{Digest, Sha256};
use studiolo_core::VaultPath;

/// Studiolo 生成副本时用的设备名；母本在 host-config（审批校验据此拒绝同名设备）。
pub const CANONICAL_DEVICE: &str = studiolo_host_config::sync_transport::CANONICAL_CONFLICT_DEVICE;
/// 冲突保全目录（范围外：`ops/` 是 SyncScope 硬排除）。
pub const CONFLICTS_DIR: &str = "ops/bridge-conflicts";
/// D5 出站前保全目录。
pub const ROLLBACK_DIR: &str = "ops/bridge-rollback";
/// 命名里的固定标记。
pub const MARKER: &str = "Conflicted copy";
/// 生成路径的字节预算（与范围索引一致）。
const PATH_MAX_BYTES: usize = 1024;
/// 生成副本时设备名的字符上限。
const DEVICE_MAX_CHARS: usize = 64;
/// 单个路径分量的字节上限（Linux NAME_MAX）。
const NAME_MAX_BYTES: usize = 255;
/// 名字过长被截短时附在截短原名后面的标记长度（`~` 加 8 位十六进制摘要）。
const SHORTENED_TAG_BYTES: usize = 9;

/// 解析出的一个冲突副本名。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictCopy {
    /// 去掉副本标记后的原路径（含扩展名；目录前缀原样保留）。
    pub original: VaultPath,
    /// 生成副本的设备名（headless 的镜像设备名，或 `studiolo-canonical`）。
    pub device: String,
    /// `yyyyMMddHHmm` 的 UTC 分钟戳。
    pub stamp: String,
}

/// 生成 `<目录>/<原路径去扩展名> (Conflicted copy <设备> <戳>).<ext>`；
/// `directory` 只能是 `CONFLICTS_DIR` 或 `ROLLBACK_DIR`。
///
/// 末段超过 255 字节（NAME_MAX）时，按字符边界截短原名、附 `~<原名摘要前 8 位>` 保证唯一（评审 P1-3）；
/// 截短后解析出的「原路径」不再等于原路径，这类副本只落在范围外的 `ops/`，不参与观察。
/// 设备名含斜杠／括号／控制字符或超过 64 字符、戳不是 12 位数字、目录不是两个固定值之一、
/// 截短也放不下、结果超过 1024 字节或不是合法（NFC）Vault 路径时返回 `None`。
#[must_use]
pub fn copy_path(
    directory: &str,
    original: &VaultPath,
    device: &str,
    stamp: &str,
) -> Option<VaultPath> {
    if (directory != CONFLICTS_DIR && directory != ROLLBACK_DIR)
        || !device_ok(device)
        || device.chars().count() > DEVICE_MAX_CHARS
        || !stamp_ok(stamp)
    {
        return None;
    }
    let (parent, name) = split_last(original.as_str());
    let (stem, ext) = match name.rfind('.') {
        Some(index) if index > 0 => name.split_at(index),
        _ => (name, ""),
    };
    let copy = fit_name(name, stem, &format!(" ({MARKER} {device} {stamp}){ext}"))?;
    let raw = match parent {
        Some(parent) => format!("{directory}/{parent}/{copy}"),
        None => format!("{directory}/{copy}"),
    };
    if raw.len() > PATH_MAX_BYTES {
        return None;
    }
    VaultPath::parse(raw).ok()
}

/// 识别 headless 或 Studiolo 生成的冲突副本；不是副本命名返回 `None`。
///
/// 只在最后一个路径分量上匹配 `<名> (Conflicted copy <设备> <12 位数字>)<.ext>`
/// （正则语义 `^(.+) \(Conflicted copy ([^()/]+) (\d{12})\)(\.[^.]+)?$`，贪婪取最长的名）；
/// 设备名可含空格，但不含 `(`、`)`、`/`。
#[must_use]
pub fn parse_copy(path: &VaultPath) -> Option<ConflictCopy> {
    let (parent, name) = split_last(path.as_str());
    let (stem, device, stamp, ext) = parse_name(name)?;
    let original = match parent {
        Some(parent) => format!("{parent}/{stem}{ext}"),
        None => format!("{stem}{ext}"),
    };
    Some(ConflictCopy {
        original: VaultPath::parse(original).ok()?,
        device: device.to_owned(),
        stamp: stamp.to_owned(),
    })
}

/// 把 Unix 秒格式化成 `yyyyMMddHHmm`（UTC）；负数或越界返回 `None`。
#[must_use]
pub fn utc_minute_stamp(unix_seconds: i64) -> Option<String> {
    if unix_seconds < 0 {
        return None;
    }
    let timestamp = jiff::Timestamp::from_second(unix_seconds).ok()?;
    let stamp = timestamp.strftime("%Y%m%d%H%M").to_string();
    stamp_ok(&stamp).then_some(stamp)
}

/// 拼出副本末段；超过 NAME_MAX 时截短原名（去扩展名部分）并附原名摘要。
fn fit_name(name: &str, stem: &str, suffix: &str) -> Option<String> {
    let whole = format!("{stem}{suffix}");
    if whole.len() <= NAME_MAX_BYTES {
        return Some(whole);
    }
    let budget = NAME_MAX_BYTES.checked_sub(suffix.len() + SHORTENED_TAG_BYTES)?;
    let cut = stem
        .char_indices()
        .map(|(index, character)| index + character.len_utf8())
        .take_while(|end| *end <= budget)
        .last()?;
    let digest = Sha256::digest(name.as_bytes());
    let tag: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(format!("{}~{tag}{suffix}", &stem[..cut]))
}

fn split_last(raw: &str) -> (Option<&str>, &str) {
    match raw.rsplit_once('/') {
        Some((parent, name)) => (Some(parent), name),
        None => (None, raw),
    }
}

/// 先按「无扩展名」（名最长）解释，再按「最后一个点之后是扩展名」解释；与正则的贪婪一致。
fn parse_name(name: &str) -> Option<(&str, &str, &str, &str)> {
    if let Some(parsed) = parse_body(name, "") {
        return Some(parsed);
    }
    let (body, ext) = name.rsplit_once('.')?;
    if ext.is_empty() {
        return None;
    }
    parse_body(body, &name[body.len()..])
}

fn parse_body<'a>(body: &'a str, ext: &'a str) -> Option<(&'a str, &'a str, &'a str, &'a str)> {
    let inner = body.strip_suffix(')')?;
    let (stem, rest) = inner.rsplit_once(&format!("({MARKER} "))?;
    let stem = stem.strip_suffix(' ')?;
    let (device, stamp) = rest.rsplit_once(' ')?;
    (!stem.is_empty() && device_ok(device) && stamp_ok(stamp)).then_some((stem, device, stamp, ext))
}

fn device_ok(device: &str) -> bool {
    !device.is_empty()
        && !device
            .chars()
            .any(|character| matches!(character, '(' | ')' | '/') || character.is_control())
}

fn stamp_ok(stamp: &str) -> bool {
    stamp.len() == 12 && stamp.bytes().all(|byte| byte.is_ascii_digit())
}
