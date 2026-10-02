//! 观察、停止与恢复类请求：probe／running／events／drain／stop／reap／match，以及 launcher
//! 重启后对旧 session 的接管。每个答案都来自实测；本文件同样不做 IO，只调 `unit.rs`。

use super::{Handler, Live};
use crate::facts::{self, CGROUP_ROOT};
use crate::protocol::{
    ContainerFrame, Empty, FRAME_MAX_BYTES, PROTOCOL_VERSION, ProbeReport, Response, StopFrame,
    Verdict,
};
use crate::session::{Phase, Session};
use crate::unit::{self, SessionDir};
use std::path::{Path, PathBuf};
use studiolo_sync_transport::owned_run::RunBudget;

pub(super) const PASSWD: &str = "/etc/passwd";
pub(super) const GROUP: &str = "/etc/group";
const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const MOUNTINFO: &str = "/proc/self/mountinfo";

impl Handler {
    pub(super) fn probe(&self, nonce: [u8; 32]) -> Result<Response, Verdict> {
        let systemd_version = self.manager.version()?;
        if systemd_version < facts::MIN_SYSTEMD_VERSION {
            return Err(unit::refused(
                "systemd 版本低于 257，建不了 PID 视图",
                Verdict::NotProvisioned,
            ));
        }
        let passwd = unit::read_text(Path::new(PASSWD)).unwrap_or_default();
        let group = unit::read_text(Path::new(GROUP)).unwrap_or_default();
        Ok(Response::Probe(ProbeReport {
            nonce,
            protocol_version: PROTOCOL_VERSION,
            systemd_version,
            cgroup_v2: unit::cgroup_v2(),
            loop_quota: self.loop_quota(),
            dedicated_identity: facts::dedicated_identity(
                &passwd,
                &group,
                self.config.payload_uid,
                self.config.payload_gid,
            ),
            payload_uid: self.config.payload_uid,
            payload_gid: self.config.payload_gid,
            boot_id: boot_id()?,
        }))
    }

    /// 镜像挂载点的块设备是 loop，且其 backing_file 正是配置的镜像文件。
    pub(super) fn loop_quota(&self) -> bool {
        let Ok(mountinfo) = unit::read_text(Path::new(MOUNTINFO)) else {
            return false;
        };
        let Some(device) = facts::mount_device(&mountinfo, &self.config.loop_mount) else {
            return false;
        };
        let backing = PathBuf::from(format!("/sys/dev/block/{device}/loop/backing_file"));
        unit::read_text(&backing)
            .is_ok_and(|text| Path::new(text.trim_end()) == self.config.loop_image)
    }

    /// launcher 重启后无记忆：只在同一 boot、unit 仍加载且 InvocationID 相同、session 目录仍在时接管；
    /// 接管的 session 直接关门，永不放行。
    pub(super) fn adopt(&mut self, frame: &ContainerFrame) -> Result<(), Verdict> {
        if boot_id()? != frame.boot_id {
            return Err(Verdict::IdentityMismatch);
        }
        let show = self
            .manager
            .show(&frame.unit_name, &["LoadState", "InvocationID"])?;
        if facts::value(&show, "LoadState") != "loaded"
            || facts::parse_hex16(facts::value(&show, "InvocationID")) != Some(frame.invocation_id)
        {
            return Err(Verdict::IdentityMismatch);
        }
        let dir = SessionDir::open(&self.sessions, &facts::hex16(&frame.launcher_session))
            .ok_or(Verdict::IdentityMismatch)?;
        let control_group = frame
            .payload_cgroup
            .strip_prefix(CGROUP_ROOT)
            .ok_or(Verdict::IdentityMismatch)?
            .to_owned();
        let report_len = unit::read_bounded(&dir.stdout(), 4 + FRAME_MAX_BYTES)
            .ok()
            .and_then(|head| facts::split_report(&head).ok().flatten())
            .map_or(0, |(_, used)| used as u64);
        let mut session = Session::barrier(frame.clone(), unknown_budget());
        session.gate();
        self.live = Some(Live {
            session,
            dir,
            gate: None,
            control_group,
            report_len,
            budget_known: false,
        });
        Ok(())
    }

    /// 垫片 exec 之后 `MainPID` 跑的是固定 node 才算开始。
    pub(super) fn running(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let unit = self.live(frame)?.session.container.unit_name.clone();
        let show = self.manager.show(&unit, &["MainPID"])?;
        let running = facts::number::<u64>(&show, "MainPID")
            .filter(|pid| *pid > 0)
            .and_then(unit::main_exe)
            .is_some_and(|exe| exe == self.config.runtime.node_path);
        Ok(Response::Running { running })
    }

    /// 组仍在就读 `cgroup.events`（并核对同一 (dev,ino)）；组已被回收且 unit 已等到主进程才算空组。
    pub(super) fn events(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let live = self.live(frame)?;
        let (unit, control_group, expected) = (
            live.session.container.unit_name.clone(),
            live.control_group.clone(),
            live.session.container.cgroup_identity,
        );
        let current = unit::stat(&unit::cgroup_dir(&control_group)).map(|(dev, ino, _)| (dev, ino));
        if current.is_some_and(|identity| identity != expected) {
            return Err(Verdict::IdentityMismatch);
        }
        if let Some((populated, frozen)) = unit::events(&control_group)? {
            return Ok(Response::Events { populated, frozen });
        }
        let show = self
            .manager
            .show(&unit, &["ControlGroup", "ExecMainCode"])?;
        let reaped = facts::value(&show, "ControlGroup").is_empty()
            && facts::number::<i32>(&show, "ExecMainCode").is_some_and(|code| code != 0);
        if reaped {
            Ok(Response::Events {
                populated: false,
                frozen: false,
            })
        } else {
            Err(unit::refused(
                "组目录不见了但 unit 未等到主进程",
                Verdict::ManagerUnavailable,
            ))
        }
    }

    /// 排空计数来自 stdout／stderr 文件大小（扣掉报告帧）；首次超限就关门、TERM、宽限后 SIGKILL。
    pub(super) fn drain(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let live = self.live(frame)?;
        let size = |path: PathBuf| unit::stat(&path).map_or(0, |(_, _, len)| len);
        let stdout_bytes = size(live.dir.stdout()).saturating_sub(live.report_len);
        let stderr_bytes = size(live.dir.stderr());
        let budget = live.session.budget;
        let over = live.budget_known
            && (stdout_bytes > u64::from(budget.stdout_bytes)
                || stderr_bytes > u64::from(budget.stderr_bytes));
        live.session.stdout_bytes = stdout_bytes;
        live.session.stderr_bytes = stderr_bytes;
        let first_overflow = over && !live.session.output_over_budget;
        if first_overflow {
            live.session.output_over_budget = true;
            live.session.gate();
            live.gate = None;
        }
        let output_over_budget = live.session.output_over_budget;
        let unit = live.session.container.unit_name.clone();
        if first_overflow {
            self.manager.kill(&unit, "SIGTERM");
            self.manager.schedule_kill(&unit, budget.stop_grace_seconds);
        }
        Ok(Response::Drain {
            stdout_bytes,
            stderr_bytes,
            output_over_budget,
        })
    }

    /// 关门并停止整组；即使已 reap 也保留 unit/session，供 owner 复验退出与静止。
    /// 只有下一次 launch 的 reap＋空组守卫通过后才退休。
    pub(super) fn stop(&mut self, frame: &StopFrame) -> Result<Response, Verdict> {
        let live = self.live(&frame.container)?;
        live.session.gate();
        live.gate = None;
        let unit = live.session.container.unit_name.clone();
        if matches!(
            self.events(&frame.container)?,
            Response::Events {
                populated: false,
                ..
            }
        ) {
            return Ok(Response::Stop(Empty {}));
        }
        self.manager.kill(&unit, "SIGTERM");
        self.manager.schedule_kill(&unit, frame.grace_seconds);
        Ok(Response::Stop(Empty {}))
    }

    /// 真实等待结果只来自 `ExecMainCode`／`ExecMainStatus`；未等到就保持 `Gated`，可重复询问。
    pub(super) fn reap(&mut self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let live = self.live(frame)?;
        if live.session.phase == Phase::Reaped {
            return Ok(Response::Reap {
                exit_observed: true,
                exit_code: live.session.exit_code,
                exit_signal: live.session.exit_signal,
            });
        }
        let unit = live.session.container.unit_name.clone();
        let show = self
            .manager
            .show(&unit, &["ExecMainCode", "ExecMainStatus"])?;
        let (exit_observed, exit_code, exit_signal) = facts::reap(&show);
        if exit_observed {
            self.live(frame)?.session.reap(exit_code, exit_signal)?;
        }
        Ok(Response::Reap {
            exit_observed,
            exit_code,
            exit_signal,
        })
    }

    /// 重开身份核对不依赖 session 记忆，全部按当前 boot、unit 与组对象实测。
    pub(super) fn matching(&self, frame: &ContainerFrame) -> Result<Response, Verdict> {
        let identity = if boot_id()? != frame.boot_id {
            "boot_changed"
        } else {
            let show = self.manager.show(
                &frame.unit_name,
                &["LoadState", "InvocationID", "ControlGroup"],
            )?;
            if facts::value(&show, "LoadState") != "loaded" {
                "unit_missing"
            } else if facts::parse_hex16(facts::value(&show, "InvocationID"))
                != Some(frame.invocation_id)
            {
                "unit_replaced"
            } else {
                let control_group = facts::value(&show, "ControlGroup");
                let current =
                    unit::stat(&unit::cgroup_dir(control_group)).map(|(dev, ino, _)| (dev, ino));
                let same_path = format!("{CGROUP_ROOT}{control_group}") == frame.payload_cgroup;
                // 组已回收（进程全退）算同一身份；路径同名但对象不同就是被复用。
                if control_group.is_empty()
                    || (same_path && current.is_none_or(|id| id == frame.cgroup_identity))
                {
                    "exact"
                } else {
                    "cgroup_reused"
                }
            }
        };
        Ok(Response::Match {
            identity: identity.to_owned(),
        })
    }
}

pub(super) fn boot_id() -> Result<[u8; 16], Verdict> {
    let text = unit::read_text(Path::new(BOOT_ID)).map_err(|_| Verdict::ManagerUnavailable)?;
    facts::parse_hex16(&text).ok_or(Verdict::ManagerUnavailable)
}

/// 接管的 session 没有预算记录；零预算占位，`budget_known=false` 让排空不做超限判断。
fn unknown_budget() -> RunBudget {
    RunBudget {
        deadline_seconds: 0,
        stop_grace_seconds: 0,
        stdout_bytes: 0,
        stderr_bytes: 0,
        config_json_bytes: 0,
        private_state_bytes: 0,
        log_bytes: 0,
        memory_bytes: 0,
        pids_max: 0,
        cpu_max_percent: 0,
    }
}
