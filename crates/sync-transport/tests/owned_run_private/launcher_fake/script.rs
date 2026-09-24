//! 假 launcher 的脚本状态机：只复现协议**可见**行为（session 一次放行、gate 后拒绝、
//! 逐字段身份核对、`Verdict` 闭集），不建 unit、不读 cgroupfs、不核隔离。

use super::super::support::{CGROUP_PREFIX, PAYLOAD_GID, PAYLOAD_UID, SERVICE_UNIT, limits};
use crate::linux::protocol::{
    ContainerFrame, Empty, IsolationFrame, LaunchFrame, LaunchReport, LimitsFrame,
    PROTOCOL_VERSION, ProbeReport, Request, Response, StopFrame, Verdict,
};
use crate::owned_run::{RunDiagnostics, RunId};

/// `probe` 应答的脚本：正常、缺某一项前置、身份或协议不合、对端被拒。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Ok,
    WithoutCgroupV2,
    WithoutLoopQuota,
    WithoutDedicatedIdentity,
    WrongIdentity,
    WrongVersion,
    WrongNonce,
    PeerRefused,
}

/// 篡改 `launch` 应答：不回显 nonce，或把容器帧里的 run_id 换掉。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tamper {
    None,
    LaunchNonce,
    LaunchRunId,
}

/// 假 launcher 的全部记忆；测试线程与应答线程经互斥锁共享。
pub struct Script {
    pub boot_id: [u8; 16],
    pub probe: Probe,
    pub tamper: Tamper,
    /// launcher 侧登记的当前 session 容器帧；`launch` 时建立。
    pub session: Option<ContainerFrame>,
    pub released: bool,
    pub gated: bool,
    pub reaped: bool,
    pub running: bool,
    pub populated: bool,
    /// `(stdout_bytes, stderr_bytes, output_over_budget)`。
    pub counts: (u64, u64, bool),
    /// `(exit_observed, exit_code, exit_signal)`。
    pub exit: (bool, Option<i32>, Option<i32>),
    pub identity: &'static str,
    pub output: Result<String, Verdict>,
    pub stops: u32,
    /// 下一条请求一律用这个分类拒绝（一次性）。
    pub refuse: Option<Verdict>,
    /// 下一条应答改发这些原始字节（一次性）；用来造撕裂帧、越界长度、越闭集载荷。
    pub raw: Option<Vec<u8>>,
    /// 已收到并成功解码的请求。
    pub requests: Vec<Request>,
    /// 已收到的原始载荷（JSON 字节）。
    pub payloads: Vec<Vec<u8>>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            boot_id: [7u8; 16],
            probe: Probe::Ok,
            tamper: Tamper::None,
            session: None,
            released: false,
            gated: false,
            reaped: false,
            running: true,
            populated: true,
            counts: (0, 0, false),
            exit: (false, None, None),
            identity: "exact",
            output: Err(Verdict::Budget),
            stops: 0,
            refuse: None,
            raw: None,
            requests: Vec::new(),
            payloads: Vec::new(),
        }
    }
}

impl Script {
    /// 按协议应答一条已解码的请求。
    pub fn respond(&mut self, request: &Request) -> Response {
        match request {
            Request::Probe { nonce } => self.probe(*nonce),
            Request::BootId(Empty {}) => Response::BootId {
                boot_id: self.boot_id,
            },
            Request::Launch(frame) => self.launch(frame),
            Request::Release(frame) => self.bound(frame, |script| {
                if script.released || script.gated {
                    return refused(Verdict::Busy);
                }
                script.released = true;
                Response::Release(Empty {})
            }),
            Request::Running(frame) => self.bound(frame, |script| Response::Running {
                running: script.running,
            }),
            Request::Gate(frame) => self.bound(frame, |script| {
                script.gated = true;
                Response::Gate(Empty {})
            }),
            Request::Events(frame) => self.bound(frame, |script| Response::Events {
                populated: script.populated,
                frozen: false,
            }),
            Request::Drain(frame) => self.bound(frame, |script| Response::Drain {
                stdout_bytes: script.counts.0,
                stderr_bytes: script.counts.1,
                output_over_budget: script.counts.2,
            }),
            Request::Stop(StopFrame {
                container,
                grace_seconds: _,
            }) => self.bound(container, |script| {
                script.stops += 1;
                Response::Stop(Empty {})
            }),
            Request::Reap(frame) => self.bound(frame, |script| {
                let (exit_observed, exit_code, exit_signal) = script.exit;
                if exit_observed {
                    script.reaped = true;
                }
                Response::Reap {
                    exit_observed,
                    exit_code,
                    exit_signal,
                }
            }),
            Request::Match(frame) => self.bound(frame, |script| Response::Match {
                identity: script.identity.to_owned(),
            }),
            Request::Output(frame) => self.bound(frame, |script| match &script.output {
                Ok(output) => Response::Output {
                    output: output.clone(),
                },
                Err(verdict) => refused(*verdict),
            }),
        }
    }

    /// 容器绑定请求：帧必须与本 session 逐字段相同，否则 `identity_mismatch`。
    fn bound(
        &mut self,
        frame: &ContainerFrame,
        reply: impl FnOnce(&mut Self) -> Response,
    ) -> Response {
        if self.session.as_ref() != Some(frame) {
            return refused(Verdict::IdentityMismatch);
        }
        reply(self)
    }

    fn probe(&self, nonce: [u8; 32]) -> Response {
        let report = ProbeReport {
            nonce,
            protocol_version: PROTOCOL_VERSION,
            systemd_version: 258,
            cgroup_v2: true,
            loop_quota: true,
            dedicated_identity: true,
            payload_uid: PAYLOAD_UID,
            payload_gid: PAYLOAD_GID,
            boot_id: self.boot_id,
        };
        Response::Probe(match self.probe {
            Probe::Ok => report,
            Probe::WithoutCgroupV2 => ProbeReport {
                cgroup_v2: false,
                ..report
            },
            Probe::WithoutLoopQuota => ProbeReport {
                loop_quota: false,
                ..report
            },
            Probe::WithoutDedicatedIdentity => ProbeReport {
                dedicated_identity: false,
                ..report
            },
            Probe::WrongIdentity => ProbeReport {
                payload_uid: PAYLOAD_UID + 1,
                ..report
            },
            Probe::WrongVersion => ProbeReport {
                protocol_version: PROTOCOL_VERSION + 1,
                ..report
            },
            Probe::WrongNonce => ProbeReport {
                nonce: [0u8; 32],
                ..report
            },
            Probe::PeerRefused => return refused(Verdict::Peer),
        })
    }

    /// 建立屏障：旧 session 未 reap 就 `busy`；容器身份由假 launcher「实测」给出。
    fn launch(&mut self, frame: &LaunchFrame) -> Response {
        if self.session.is_some() && !self.reaped {
            return refused(Verdict::Busy);
        }
        let container = ContainerFrame {
            run_id: frame.run_id,
            boot_id: self.boot_id,
            unit_name: SERVICE_UNIT.to_owned(),
            invocation_id: [8u8; 16],
            payload_cgroup: format!("{CGROUP_PREFIX}/run-{}", frame.sequence),
            cgroup_identity: (64, 99),
            launcher_session: [9u8; 16],
        };
        self.session = Some(container.clone());
        self.released = false;
        self.gated = false;
        self.reaped = false;
        let actual = limits();
        let mut report = LaunchReport {
            nonce: frame.nonce,
            container,
            limits: LimitsFrame {
                memory_max: actual.memory_max,
                memory_swap_max: actual.memory_swap_max,
                pids_max: actual.pids_max,
                cpu_max: actual.cpu_max,
            },
            isolation: IsolationFrame {
                mount_namespace: true,
                pid_namespace: true,
                dedicated_identity: true,
                dropped_capabilities: true,
                payload_cannot_write_cgroup: true,
                final_view_identity_checked: true,
                disk_quota_enforced: true,
            },
            diagnostics: RunDiagnostics {
                pid: Some(4242),
                start_time: Some(1),
            },
        };
        match self.tamper {
            Tamper::None => {}
            Tamper::LaunchNonce => report.nonce = [0u8; 32],
            Tamper::LaunchRunId => report.container.run_id = RunId::from_bytes([0xAA; 16]),
        }
        Response::Launch(report)
    }
}

fn refused(verdict: Verdict) -> Response {
    Response::Refused { verdict }
}
