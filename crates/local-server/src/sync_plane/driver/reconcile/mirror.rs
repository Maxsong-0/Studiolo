//! 重开核对的镜像侧：静止窗口内经视图逐帧读目标，与冻结源／前置版本逐字节比对。
//!
//! 本文件不打开路径：读法就是执行器取正文的 `PlaneContent`（打开时核字节数，读完核摘要），
//! 缺失、不符与读不出各自分开，读不出一律按「其它」保持未知。

use super::super::content::{FRAME_BYTES, RoundContext};
use super::MirrorTarget;
use crate::sync_plane::mirror::PlaneContent;
use studiolo_core::VaultPath;
use studiolo_sync_bridge::executor::{MirrorContent, TransferFault};
use studiolo_sync_bridge::{Action, Change, FileVersion, Side};

/// 目标相对单个冻结版本的比对结果。
enum Probe {
    Equal,
    Missing,
    Differs,
    Unreadable,
}

/// 目标相对冻结源（先比）与冻结前置的现状；两者都不等时，缺失即 `Missing`，否则 `Other`。
pub(super) fn target(context: &RoundContext<'_>, action: &Action) -> MirrorTarget {
    let (source, expected) = match &action.change {
        Change::CreateOnly { source } => (Some(source), None),
        Change::Replace {
            source,
            target_expected,
        } => (Some(source), Some(target_expected)),
        Change::Delete { target_expected } => (None, Some(target_expected)),
    };
    let mut missing = false;
    for (version, hit) in [
        (source, MirrorTarget::Source),
        (expected, MirrorTarget::Expected),
    ] {
        let Some(version) = version else {
            continue;
        };
        match probe(context, &action.key.path, version) {
            Probe::Equal => return hit,
            Probe::Missing => missing = true,
            Probe::Differs => {}
            Probe::Unreadable => return MirrorTarget::Other,
        }
    }
    if missing {
        MirrorTarget::Missing
    } else {
        MirrorTarget::Other
    }
}

/// 经视图打开并读完目标，与一个冻结版本比对。
fn probe(context: &RoundContext<'_>, path: &VaultPath, version: &FileVersion) -> Probe {
    let mut content = PlaneContent::new(context.plane, &context.view);
    match content.open(Side::Mirror, path, version) {
        Ok(()) => {}
        Err(TransferFault::SourceMissing) => return Probe::Missing,
        Err(TransferFault::ContentMismatch) => return Probe::Differs,
        Err(_) => return Probe::Unreadable,
    }
    let mut buffer = vec![0u8; FRAME_BYTES];
    loop {
        match content.next_frame(&mut buffer) {
            Ok(0) => return Probe::Equal,
            Ok(_) => {}
            Err(TransferFault::ContentMismatch | TransferFault::Truncated) => {
                return Probe::Differs;
            }
            Err(_) => return Probe::Unreadable,
        }
    }
}
