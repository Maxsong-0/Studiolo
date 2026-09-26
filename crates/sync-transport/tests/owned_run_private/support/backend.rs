//! 脚本化后端与容器夹具：从 `support.rs` 拆出，只为压住单文件行数上限。
//!
//! `FakeBackend` 只驱动纯状态机与持久层，拿不到 `VerifiedLinuxBackend`。

use super::*;

pub fn container() -> ContainerIdentity {
    ContainerIdentity {
        boot_id: [7u8; 16],
        unit_name: SERVICE_UNIT.to_owned(),
        invocation_id: [8u8; 16],
        payload_cgroup: format!("{CGROUP_PREFIX}/run-1"),
        cgroup_identity: (64, 99),
        launcher_session: [9u8; 16],
    }
}

pub fn limits() -> CgroupLimits {
    CgroupLimits {
        memory_max: Some(2_147_483_648),
        memory_swap_max: Some(0),
        pids_max: Some(64),
        cpu_max: Some((200_000, 100_000)),
    }
}

pub fn isolation() -> IsolationReport {
    IsolationReport {
        mount_namespace: true,
        pid_namespace: true,
        dedicated_identity: true,
        dropped_capabilities: true,
        payload_cannot_write_cgroup: true,
        final_view_identity_checked: true,
        disk_quota_enforced: true,
    }
}

/// 脚本化后端；`launches`/`releases` 计数用来证明绝不二次 spawn。
pub struct FakeBackend {
    pub boot: [u8; 16],
    pub container: ContainerIdentity,
    pub limits: CgroupLimits,
    pub isolation: IsolationReport,
    pub populated: bool,
    pub running: bool,
    pub exit: ExitObservation,
    pub counts: ResourceCounts,
    pub identity: IdentityMatch,
    pub launches: u32,
    pub releases: u32,
    pub stops: u32,
    pub gate_closures: u32,
    pub launch_gap: Option<ContainmentGap>,
    pub release_gap: Option<ContainmentGap>,
    pub observe_gap: Option<ContainmentGap>,
    /// `config_output` 返回的正文；`None` 即 launcher 报超预算。
    pub output: Option<String>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self {
            boot: [7u8; 16],
            container: container(),
            limits: limits(),
            isolation: isolation(),
            populated: true,
            running: true,
            exit: ExitObservation::NotObserved,
            counts: ResourceCounts::default(),
            identity: IdentityMatch::Exact,
            launches: 0,
            releases: 0,
            stops: 0,
            gate_closures: 0,
            launch_gap: None,
            release_gap: None,
            observe_gap: None,
            output: None,
        }
    }
}

impl ContainmentBackend for FakeBackend {
    fn boot_id(&mut self) -> std::result::Result<[u8; 16], ContainmentGap> {
        Ok(self.boot)
    }

    fn issue_launch(
        &mut self,
        request: &LaunchRequest,
    ) -> std::result::Result<LaunchAcknowledgement, ContainmentGap> {
        self.launches += 1;
        if let Some(gap) = self.launch_gap {
            return Err(gap);
        }
        Ok(LaunchAcknowledgement {
            container: self.container.clone(),
            limits: self.limits,
            isolation: self.isolation,
            diagnostics: RunDiagnostics::default(),
            release: ReleaseToken::new(request.run_id, self.container.launcher_session),
        })
    }

    fn release_once(&mut self, _token: ReleaseToken) -> std::result::Result<(), ContainmentGap> {
        self.releases += 1;
        self.release_gap.map_or(Ok(()), Err)
    }

    fn observe_running(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<bool, ContainmentGap> {
        self.observe_gap.map_or(Ok(self.running), Err)
    }

    fn close_spawn_gate(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<(), ContainmentGap> {
        self.gate_closures += 1;
        Ok(())
    }

    fn read_events(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<CgroupEvents, ContainmentGap> {
        Ok(CgroupEvents {
            populated: self.populated,
            frozen: false,
        })
    }

    fn drain(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<ResourceCounts, ContainmentGap> {
        Ok(self.counts)
    }

    fn stop(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
        _grace_seconds: u32,
    ) -> std::result::Result<(), ContainmentGap> {
        self.stops += 1;
        Ok(())
    }

    fn reap(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<ExitObservation, ContainmentGap> {
        Ok(self.exit)
    }

    fn match_identity(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<IdentityMatch, ContainmentGap> {
        Ok(self.identity)
    }

    fn config_output(
        &mut self,
        _run_id: RunId,
        _container: &ContainerIdentity,
    ) -> std::result::Result<String, ContainmentGap> {
        let output = self.output.clone();
        output.ok_or(ContainmentGap::BudgetUnsupported)
    }
}
