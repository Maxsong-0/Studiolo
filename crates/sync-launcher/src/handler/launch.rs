//! launch 的建立流程：起 unit → 等垫片报告 → 回读属性与 cgroup 事实 → 七项隔离 → launch 应答。
//! 任一步失败都由 `handler.rs` 的 `launch` 统一清场（SIGKILL、卸载、删目录）。

use super::Handler;
use super::observe::{GROUP, PASSWD, boot_id};
use crate::facts::{self, CGROUP_ROOT, IsolationInputs};
use crate::protocol::{
    ContainerFrame, FRAME_MAX_BYTES, LaunchFrame, LaunchReport, PayloadReport, Verdict,
};
use crate::unit::{self, SessionDir};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use studiolo_sync_transport::owned_run::RunDiagnostics;

/// 垫片报告帧的等待上限。
const REPORT_WAIT: Duration = Duration::from_secs(10);
/// launch 应答需要回读的属性。
const LAUNCH_PROPERTIES: [&str; 16] = [
    "InvocationID",
    "ControlGroup",
    "MainPID",
    "ExecMainStartTimestampMonotonic",
    "ExecMainCode",
    "User",
    "Group",
    "SupplementaryGroups",
    "CapabilityBoundingSet",
    "PrivateMounts",
    "PrivatePIDs",
    "ProtectSystem",
    "ProtectHome",
    "ProtectControlGroups",
    "RestrictAddressFamilies",
    "InaccessiblePaths",
];

impl Handler {
    /// 起 unit、等垫片报告、回读属性与 cgroup 事实，拼出 launch 应答；返回值还带组路径与报告帧长度。
    pub(super) fn establish(
        &self,
        frame: &LaunchFrame,
        unit: &str,
        dir: &SessionDir,
        session_id: [u8; 16],
    ) -> Result<(LaunchReport, String, u64), Verdict> {
        self.manager.run_transient(&self.config, unit, dir, frame)?;
        let (report, report_len) = self.wait_report(unit, dir)?;
        let config = &self.config;
        let identity = |path: &Path| {
            unit::stat(path)
                .map(|(dev, ino, _)| (dev, ino))
                .ok_or_else(|| {
                    unit::refused(
                        &format!("launcher 视图内 stat 不到 {}", path.display()),
                        Verdict::NotProvisioned,
                    )
                })
        };
        let own = [
            identity(&config.mirror_root)?,
            identity(&config.payload_config_root)?,
            identity(&config.runtime.node_path)?,
            identity(&config.runtime.cli_path)?,
        ];
        let show = self.manager.show(unit, &LAUNCH_PROPERTIES)?;
        let invocation_id = facts::parse_hex16(facts::value(&show, "InvocationID"))
            .ok_or_else(|| unit::refused("InvocationID 回读不合法", Verdict::ManagerUnavailable))?;
        let control_group = facts::value(&show, "ControlGroup").to_owned();
        if control_group.is_empty() {
            return Err(unit::refused(
                "ControlGroup 回读为空",
                Verdict::ManagerUnavailable,
            ));
        }
        if !facts::within_prefix(&control_group, &config.payload_cgroup_prefix) {
            let step = format!(
                "实测组 {control_group} 不在批准前缀 {} 之内",
                config.payload_cgroup_prefix.display()
            );
            return Err(unit::refused(&step, Verdict::NotProvisioned));
        }
        let cgroup_identity = unit::stat(&unit::cgroup_dir(&control_group))
            .map(|(dev, ino, _)| (dev, ino))
            .ok_or_else(|| {
                unit::refused("stat 不到 payload 子组目录", Verdict::ManagerUnavailable)
            })?;
        let limits = unit::limits(&control_group)?;
        let passwd = unit::read_text(Path::new(PASSWD)).unwrap_or_default();
        let group = unit::read_text(Path::new(GROUP)).unwrap_or_default();
        let payload_user = facts::passwd_name(&passwd, config.payload_uid, config.payload_gid);
        let payload_group = facts::group_name(&group, config.payload_gid);
        let isolation = facts::isolation(&IsolationInputs {
            show: &show,
            report: &report,
            own,
            payload_uid: config.payload_uid,
            payload_gid: config.payload_gid,
            payload_user: payload_user.as_deref(),
            payload_group: payload_group.as_deref(),
            inaccessible: &config.inaccessible,
            loop_quota: self.loop_quota(),
        });
        let container = ContainerFrame {
            run_id: frame.run_id,
            boot_id: boot_id()?,
            unit_name: unit.to_owned(),
            invocation_id,
            payload_cgroup: format!("{CGROUP_ROOT}{control_group}"),
            cgroup_identity,
            launcher_session: session_id,
        };
        let diagnostics = RunDiagnostics {
            pid: facts::number::<i32>(&show, "MainPID").filter(|pid| *pid > 0),
            start_time: facts::number::<u64>(&show, "ExecMainStartTimestampMonotonic"),
        };
        let report = LaunchReport {
            nonce: frame.nonce,
            container,
            limits,
            isolation,
            diagnostics,
        };
        Ok((report, control_group, report_len))
    }

    /// 轮询 stdout 头部直到一帧完整的报告；垫片先退出或超时都算失败。
    fn wait_report(&self, unit: &str, dir: &SessionDir) -> Result<(PayloadReport, u64), Verdict> {
        let deadline = Instant::now() + REPORT_WAIT;
        let mut polls: u32 = 0;
        loop {
            if let Ok(head) = unit::read_bounded(&dir.stdout(), 4 + FRAME_MAX_BYTES)
                && let Some((report, used)) = facts::split_report(&head)?
            {
                return Ok((report, used as u64));
            }
            polls += 1;
            if polls.is_multiple_of(10) {
                let show = self.manager.show(unit, &["ExecMainCode"])?;
                if facts::number::<i32>(&show, "ExecMainCode").is_some_and(|code| code != 0) {
                    return Err(unit::refused("垫片报告前已退出", Verdict::Protocol));
                }
            }
            if Instant::now() >= deadline {
                return Err(unit::refused("等垫片报告超时", Verdict::ManagerUnavailable));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
