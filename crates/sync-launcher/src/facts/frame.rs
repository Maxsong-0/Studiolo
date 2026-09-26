//! launcher ↔ 垫片之间的两种帧（都不是 owner IPC）：控制 FIFO 上的启动事实帧，
//! 与 stdout 文件头部的报告帧。帧格式复用 `ipc::{encode_frame,decode_frame}`。

use crate::protocol::{
    FRAME_MAX_BYTES, PayloadReport, Verdict, decode_frame, decode_payload_report, encode_frame,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use studiolo_sync_transport::owned_run::{RunOperation, RuntimePins};

/// launcher 经控制 FIFO 交给垫片的启动事实：固定 pins、镜像根、配置根与操作。
/// 它来自 launcher 自己的 0600 配置，不来自 IPC；不含任何凭据。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchFacts {
    /// 固定为 `launch_facts`。
    pub kind: LaunchFactsKind,
    /// 固定运行时（node、cli.js、版本、闭包摘要）。
    pub runtime: RuntimePins,
    /// payload 视图内的真实镜像根。
    pub mirror_root: PathBuf,
    /// headless 配置根（`XDG_CONFIG_HOME`）。
    pub payload_config_root: PathBuf,
    /// 本次运行的固定操作。
    pub operation: RunOperation,
}

/// `LaunchFacts.kind` 的单值枚举。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchFactsKind {
    /// 唯一值。
    LaunchFacts,
}

/// 编码启动事实帧（同一 4 字节长度前缀，≤16 KiB）。
///
/// # Errors
/// 序列化失败返回 `Protocol`；超帧返回 `Budget`。
pub fn encode_launch_facts(facts: &LaunchFacts) -> Result<Vec<u8>, Verdict> {
    let payload = serde_json::to_vec(facts).map_err(|_| Verdict::Protocol)?;
    encode_frame(&payload).map_err(|_| Verdict::Budget)
}

/// 解码启动事实帧。
///
/// # Errors
/// 帧或形状不合法返回 `Protocol`。
pub fn decode_launch_facts(frame: &[u8]) -> Result<LaunchFacts, Verdict> {
    let payload = decode_frame(frame).map_err(|_| Verdict::Protocol)?;
    serde_json::from_slice(payload).map_err(|_| Verdict::Protocol)
}

/// stdout 文件头部的报告帧：字节不够返回 `Ok(None)`；长度超帧上限或形状不对是协议错误。
/// 返回值里的 `usize` 是报告帧占用的字节数，之后才是 payload 自己的输出。
///
/// # Errors
/// 长度前缀超上限或报告形状不合法返回 `Protocol`。
pub fn split_report(stdout: &[u8]) -> Result<Option<(PayloadReport, usize)>, Verdict> {
    let Some(header) = stdout.get(..4) else {
        return Ok(None);
    };
    let header: [u8; 4] = header.try_into().map_err(|_| Verdict::Protocol)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > FRAME_MAX_BYTES {
        return Err(Verdict::Protocol);
    }
    let Some(frame) = stdout.get(..4 + length) else {
        return Ok(None);
    };
    decode_payload_report(frame).map(|report| Some((report, 4 + length)))
}
