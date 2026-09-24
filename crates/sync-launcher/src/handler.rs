//! 请求编排：每个请求映射成 `unit.rs` 的真实读写与 `session.rs` 的状态迁移；本文件不做 IO。
//! launch 的拒绝顺序按设计：`Busy`（旧 session 未 reap 或组仍有进程）→ `ArgvMismatch` → 建 session。
//! 建立流程在 `handler/launch.rs`；观察、停止与 launcher 重启后的接管在 `handler/observe.rs`。

mod launch;
mod observe;

use crate::config::LauncherConfig;
use crate::facts::{self, LaunchFacts, LaunchFactsKind};
use crate::protocol::{ContainerFrame, Empty, LaunchFrame, Request, Response, Verdict};
use crate::session::{Phase, Session};
use crate::unit::{self, Manager, ReleaseGate, SessionDir};
use observe::boot_id;
use std::path::PathBuf;
use studiolo_sync_transport::linux::service;

/// 一个服务进程的全部记忆：固定配置、manager、sessions 目录与至多一个未退休的 session。
pub struct Handler {
    config: LauncherConfig,
    manager: Manager,
    sessions: PathBuf,
    live: Option<Live>,
}

/// 未退休的 session：状态机、目录、控制 FIFO 写端与 launch 时记下的组路径。
struct Live {
    session: Session,
    dir: SessionDir,
    gate: Option<ReleaseGate>,
    control_group: String,
    /// stdout 头部报告帧的字节数；排空与正文都从它之后算。
    report_len: u64,
    /// launcher 重启后接管的 session 不知道预算，排空只报计数不做超限停止。
    budget_known: bool,
}

impl Handler {
    /// 固定配置与 manager 一起构造；`sessions` 是 session 目录的父目录。
    #[must_use]
    pub fn new(config: LauncherConfig, manager: Manager, sessions: PathBuf) -> Self {
        Self {
            config,
            manager,
            sessions,
            live: None,
        }
    }

    /// 固定配置（socket 层核对对端凭据用）。
    #[must_use]
    pub fn config(&self) -> &LauncherConfig {
        &self.config
    }

    /// 处理一帧请求；任何拒绝都只带固定 `Verdict`。
    pub fn handle(&mut self, request: Request) -> Response {
        let result = match request {
            Request::Probe { nonce } => self.probe(nonce),
            Request::BootId(_) => boot_id().map(|boot_id| Response::BootId { boot_id }),
            Request::Launch(frame) => self.launch(&frame),
            Request::Release(frame) => self.release(&frame),
            Request::Running(frame) => self.running(&frame),
            Request::Gate(frame) => self.gate(&frame),
            Request::Events(frame) => self.events(&frame),
            Request::Drain(frame) => self.drain(&frame),
            Request::Stop(frame) => self.stop(&frame),
            Request::Reap(frame) => self.reap(&frame),
            Request::Match(frame) => self.matching(&frame),
            Request::Output(frame) => self.output(&frame),
        };
        result.unwrap_or_else(|verdict| Response::Refused { verdict })
    }

    fn launch(&mut self, frame: &LaunchFrame) -> Result<Response, Verdict> {
        if !frame.budget.complete() {
            return Err(Verdict::Budget);
        }
        if frame.payload_uid != self.config.payload_uid
            || frame.payload_gid != self.config.payload_gid
        {
            return Err(unit::refused(
                "请求的 payload 身份与固定配置不符",
                Verdict::NotProvisioned,
            ));
        }
        self.retire_replaceable()?;
        let operation = facts::operation(frame)?;
        let argv = service::build_argv(&self.config.runtime, operation, &self.config.mirror_root)
            .map_err(|_| Verdict::Protocol)?;
        if service::argv_digest(&argv) != frame.argv_digest {
            return Err(Verdict::ArgvMismatch);
        }
        let mut session_id = [0u8; 16];
        getrandom::fill(&mut session_id).map_err(|_| Verdict::ManagerUnavailable)?;
        let unit = facts::unit_name(frame.sequence, &frame.run_id);
        let (dir, mut gate) = SessionDir::create(&self.sessions, &facts::hex16(&session_id))?;
        let launch_facts = LaunchFacts {
            kind: LaunchFactsKind::LaunchFacts,
            runtime: self.config.runtime.clone(),
            mirror_root: self.config.mirror_root.clone(),
            payload_config_root: self.config.payload_config_root.clone(),
            operation,
        };
        // 事实帧先进 FIFO，垫片一起来就能读到；之后才起 unit。
        let established = facts::encode_launch_facts(&launch_facts)
            .and_then(|bytes| gate.write(&bytes))
            .and_then(|()| self.establish(frame, &unit, &dir, session_id));
        match established {
            Ok((report, control_group, report_len)) => {
                self.live = Some(Live {
                    session: Session::barrier(report.container.clone(), frame.budget),
                    dir,
                    gate: Some(gate),
                    control_group,
                    report_len,
                    budget_known: true,
                });
                Ok(Response::Launch(report))
            }
            Err(verdict) => {
                drop(gate);
                self.manager.kill(&unit, "SIGKILL");
                self.manager.unload(&unit);
                dir.remove();
                Err(verdict)
            }
        }
    }

    /// 旧 session 只有已 reap 且组已空才能让位；否则 `Busy`。
    fn retire_replaceable(&mut self) -> Result<(), Verdict> {
        let Some(live) = self.live.as_ref() else {
            return Ok(());
        };
        if !live.session.replaceable() {
            return Err(Verdict::Busy);
        }
        if unit::events(&live.control_group)?.is_some_and(|(populated, _)| populated) {
            return Err(Verdict::Busy);
        }
        self.retire();
        Ok(())
    }

    /// 退休：关门、卸载 unit、删 session 目录。
    fn retire(&mut self) {
        if let Some(live) = self.live.take() {
            drop(live.gate);
            self.manager.unload(&live.session.container.unit_name);
            live.dir.remove();
        }
    }

    /// 请求帧对应的 session：没有记忆时尝试接管，然后逐字段核对身份。
    fn live(&mut self, frame: &ContainerFrame) -> Result<&mut Live, Verdict> {
        if self.live.is_none() {
            self.adopt(frame)?;
        }
        let live = self.live.as_mut().ok_or(Verdict::IdentityMismatch)?;
        live.session.check(frame)?;
        Ok(live)
    }

    fn release(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let live = self.live(frame)?;
        live.session.release()?;
        live.gate.as_mut().ok_or(Verdict::Busy)?.write(b"r")?;
        Ok(Response::Release(Empty {}))
    }

    fn gate(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let live = self.live(frame)?;
        live.session.gate();
        live.gate = None;
        Ok(Response::Gate(Empty {}))
    }

    /// 已 reap 的 InspectConfig／SetMode 的 stdout 正文：跳过报告帧，限 `config_json_bytes` 且须 UTF-8。
    fn output(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let live = self.live(frame)?;
        if live.session.phase != Phase::Reaped {
            return Err(Verdict::Busy);
        }
        let skip = live.report_len as usize;
        let cap = skip + live.session.budget.config_json_bytes as usize;
        let bytes = unit::read_bounded(&live.dir.stdout(), cap).map_err(|_| Verdict::Budget)?;
        let body = bytes.get(skip..).unwrap_or_default();
        let output = String::from_utf8(body.to_vec()).map_err(|_| Verdict::Budget)?;
        Ok(Response::Output { output })
    }
}
