//! launcher IPC 的纯编解码：私有端点、有界帧、封闭字段；不传 secret、正文或任意路径。

use super::ContainmentGap;

/// 单帧载荷硬上限。
pub const FRAME_MAX_BYTES: usize = 16 * 1024;

/// 对端凭据；必须来自内核报告的实际 peer credentials。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerCredentials {
    /// 对端真实 UID。
    pub uid: u32,
    /// 对端真实 GID。
    pub gid: u32,
    /// 对端 PID；仅诊断。
    pub pid: i32,
}

/// 帧载荷允许出现的全部顶层字段。
const ALLOWED_KEYS: [&str; 10] = [
    "run_id",
    "sequence",
    "operation",
    "nonce",
    "unit_name",
    "invocation_id",
    "payload_cgroup",
    "cgroup_identity",
    "launcher_session",
    "verdict",
];

/// 编码一帧：4 字节大端长度前缀加载荷。
///
/// # Errors
/// 载荷超过 16 KiB 返回 `FrameTooLarge`。
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, ContainmentGap> {
    if payload.len() > FRAME_MAX_BYTES || payload.is_empty() {
        return Err(ContainmentGap::FrameTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ContainmentGap::FrameTooLarge)?;
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// 解码一帧；长度必须与实际字节数完全一致，不接受粘包或截断。
///
/// # Errors
/// 长度不符返回 `ProtocolViolation`；超限返回 `FrameTooLarge`。
pub fn decode_frame(bytes: &[u8]) -> Result<&[u8], ContainmentGap> {
    let header: [u8; 4] = bytes
        .get(..4)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(ContainmentGap::ProtocolViolation)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > FRAME_MAX_BYTES {
        return Err(ContainmentGap::FrameTooLarge);
    }
    let payload = bytes.get(4..).ok_or(ContainmentGap::ProtocolViolation)?;
    if payload.len() != length {
        return Err(ContainmentGap::ProtocolViolation);
    }
    Ok(payload)
}

/// 载荷必须是封闭字段的单个 JSON 对象；未知顶层键一律拒绝。
///
/// # Errors
/// 非对象、含未知键或嵌套过深时返回 `ProtocolViolation`。
pub fn payload_acceptable(payload: &[u8]) -> Result<(), ContainmentGap> {
    let value: serde_json::Value =
        serde_json::from_slice(payload).map_err(|_| ContainmentGap::ProtocolViolation)?;
    let object = value.as_object().ok_or(ContainmentGap::ProtocolViolation)?;
    if object.is_empty() {
        return Err(ContainmentGap::ProtocolViolation);
    }
    for key in object.keys() {
        if !ALLOWED_KEYS.contains(&key.as_str()) {
            return Err(ContainmentGap::ProtocolViolation);
        }
    }
    Ok(())
}

/// 对端必须正是批准的 payload 身份；只看自报字段不算。
///
/// # Errors
/// 身份不符返回 `PeerRejected`。
pub fn peer_acceptable(
    peer: &PeerCredentials,
    expected_uid: u32,
    expected_gid: u32,
) -> Result<(), ContainmentGap> {
    if peer.uid == expected_uid && peer.gid == expected_gid && peer.uid != 0 {
        Ok(())
    } else {
        Err(ContainmentGap::PeerRejected)
    }
}
