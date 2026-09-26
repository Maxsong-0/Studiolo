//! 线上契约的重导出与 launcher 侧的编解码入口；DTO 母本在 sync-transport，这里不再定义第二份。

pub use studiolo_sync_transport::linux::ipc::{
    ALLOWED_KEYS, FRAME_MAX_BYTES, PeerCredentials, decode_frame, encode_frame, payload_acceptable,
    peer_acceptable,
};
pub use studiolo_sync_transport::linux::protocol::{
    ALL_KEYS, ContainerFrame, Empty, IsolationFrame, LAUNCHER_SOCKET, LaunchFrame, LaunchReport,
    LimitsFrame, PROTOCOL_VERSION, PayloadReport, PayloadReportKind, ProbeReport, Request,
    Response, StopFrame, Verdict,
};

/// launcher 侧解码一帧请求：先过 `ipc::decode_frame` 与顶层键闭集，再反序列化。
///
/// # Errors
/// 帧长不符、未知键或形状不合法时返回 `Verdict::Protocol`。
pub fn decode_request(frame: &[u8]) -> Result<Request, Verdict> {
    let payload = decode_frame(frame).map_err(|_| Verdict::Protocol)?;
    payload_acceptable(payload).map_err(|_| Verdict::Protocol)?;
    serde_json::from_slice(payload).map_err(|_| Verdict::Protocol)
}

/// launcher 侧编码一帧应答；超过帧上限即 `Verdict::Budget`（应答方改回 `Refused`）。
///
/// # Errors
/// 序列化失败返回 `Verdict::Protocol`；超帧返回 `Verdict::Budget`。
pub fn encode_response(response: &Response) -> Result<Vec<u8>, Verdict> {
    let payload = serde_json::to_vec(response).map_err(|_| Verdict::Protocol)?;
    encode_frame(&payload).map_err(|_| Verdict::Budget)
}

/// 垫片侧编码报告帧；只有一种形状。
///
/// # Errors
/// 序列化失败返回 `Verdict::Protocol`。
pub fn encode_payload_report(report: &PayloadReport) -> Result<Vec<u8>, Verdict> {
    let payload = serde_json::to_vec(report).map_err(|_| Verdict::Protocol)?;
    encode_frame(&payload).map_err(|_| Verdict::Budget)
}

/// launcher 侧解码垫片报告；顶层键同样受闭集约束。
///
/// # Errors
/// 帧或形状不合法返回 `Verdict::Protocol`。
pub fn decode_payload_report(frame: &[u8]) -> Result<PayloadReport, Verdict> {
    let payload = decode_frame(frame).map_err(|_| Verdict::Protocol)?;
    payload_acceptable(payload).map_err(|_| Verdict::Protocol)?;
    serde_json::from_slice(payload).map_err(|_| Verdict::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 协议用到的每个顶层键都在帧闭集里() {
        for key in ALL_KEYS {
            assert!(
                ALLOWED_KEYS.contains(&key),
                "{key} 不在 ipc::ALLOWED_KEYS 里"
            );
        }
    }

    #[test]
    fn 每种请求编码后都能过闭集并解回同一值() {
        let container = ContainerFrame {
            run_id: studiolo_sync_transport::owned_run::RunId::from_bytes([1u8; 16]),
            boot_id: [2u8; 16],
            unit_name: "studiolo-sync-payload-7-01010101.service".to_owned(),
            invocation_id: [3u8; 16],
            payload_cgroup: "/sys/fs/cgroup/system.slice/x.service".to_owned(),
            cgroup_identity: (64, 99),
            launcher_session: [4u8; 16],
        };
        let requests = [
            Request::Probe { nonce: [5u8; 32] },
            Request::BootId(Empty {}),
            Request::Release(container.clone()),
            Request::Running(container.clone()),
            Request::Gate(container.clone()),
            Request::Events(container.clone()),
            Request::Drain(container.clone()),
            Request::Stop(StopFrame {
                container: container.clone(),
                grace_seconds: 30,
            }),
            Request::Reap(container.clone()),
            Request::Match(container.clone()),
            Request::Output(container),
        ];
        for request in requests {
            let payload = serde_json::to_vec(&request).expect("可序列化");
            let frame = encode_frame(&payload).expect("可成帧");
            assert_eq!(decode_request(&frame).expect("可解码"), request);
        }
    }

    #[test]
    fn 带可执行路径或凭据的帧一律被拒() {
        for bad in [
            br#"{"kind":"launch","argv":["/bin/sh"]}"#.as_slice(),
            br#"{"kind":"launch","auth_token":"x"}"#.as_slice(),
            br#"{"kind":"launch","mirror_path":"/srv"}"#.as_slice(),
            br#"{"kind":"unknown_kind"}"#.as_slice(),
        ] {
            let frame = encode_frame(bad).expect("可成帧");
            assert_eq!(decode_request(&frame).err(), Some(Verdict::Protocol));
        }
    }
}
