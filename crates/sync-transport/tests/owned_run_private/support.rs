//! 纯内存夹具：假批准、假持久层、脚本化后端。
//!
//! 这里的后端**只**用来驱动纯状态机与持久层；它拿不到 `VerifiedLinuxBackend`，
//! 因此任何测试都不能把平台标记为「受控隔离可用」。

use crate::linux::cgroup::{CgroupEvents, CgroupLimits};
use crate::linux::launcher::{IsolationReport, LaunchAcknowledgement, LaunchRequest, ReleaseToken};
use crate::linux::{ContainmentBackend, ContainmentGap, IdentityMatch};
use crate::owned_run::session::RunSession;
use crate::owned_run::*;
use crate::store::StatePublisher;
use std::{cell::RefCell, num::NonZeroU64, path::PathBuf, rc::Rc};
use studiolo_core::VaultUuid;

pub const PRIVATE_ROOT: &str = "/srv/studiolo/transport";
pub const MIRROR_ROOT: &str = "/srv/studiolo/mirror";
pub const SERVICE_UNIT: &str = "studiolo-sync-launcher@link-1.service";
pub const CGROUP_PREFIX: &str = "/sys/fs/cgroup/studiolo.slice/payload";
pub const OWNER: OwnerIdentity = OwnerIdentity {
    uid: 1000,
    gid: 1000,
};

/// 假批准：字段可改，用来模拟配置漂移与根替换。
pub struct FakeApproval {
    pub fresh: RefCell<bool>,
    pub binding: RefCell<TransportBindingFacts>,
    pub roots: RefCell<RootPins>,
    pub runtime: RuntimePins,
    pub backend: RefCell<BackendDescriptor>,
    pub budget: RefCell<RunBudget>,
}

impl FakeApproval {
    pub fn new() -> Self {
        Self {
            fresh: RefCell::new(true),
            binding: RefCell::new(TransportBindingFacts {
                vault_uuid: VaultUuid::new(),
                link_id: [1u8; 16],
                remote_vault: [2u8; 16],
                scope_epoch: NonZeroU64::new(1).expect("范围代次非零"),
                scope_hash: [3u8; 32],
                approved_config_digest: [4u8; 32],
            }),
            roots: RefCell::new(RootPins {
                private_root: PathBuf::from(PRIVATE_ROOT),
                private_identity: (64, 11),
                mirror_root: PathBuf::from(MIRROR_ROOT),
                mirror_identity: (64, 12),
            }),
            runtime: RuntimePins {
                version: PINNED_RUNTIME_VERSION.to_owned(),
                node_path: PathBuf::from("/opt/studiolo/node/bin/node"),
                cli_path: PathBuf::from("/opt/studiolo/obsidian-headless/cli.js"),
                closure_digest: [5u8; 32],
            },
            backend: RefCell::new(BackendDescriptor {
                service_unit: SERVICE_UNIT.to_owned(),
                payload_uid: 61001,
                payload_gid: 61001,
                payload_cgroup_prefix: CGROUP_PREFIX.to_owned(),
                digest: [6u8; 32],
            }),
            budget: RefCell::new(budget()),
        }
    }
}

impl ApprovedTransport for FakeApproval {
    fn revalidate(&self) -> Result<()> {
        if *self.fresh.borrow() {
            Ok(())
        } else {
            Err(TransportError::ConfigurationChanged)
        }
    }

    fn binding(&self) -> Result<TransportBindingFacts> {
        Ok(self.binding.borrow().clone())
    }

    fn roots(&self) -> RootPins {
        self.roots.borrow().clone()
    }

    fn runtime(&self) -> RuntimePins {
        self.runtime.clone()
    }

    fn backend(&self) -> BackendDescriptor {
        self.backend.borrow().clone()
    }

    fn budget(&self) -> RunBudget {
        *self.budget.borrow()
    }
}

pub fn budget() -> RunBudget {
    RunBudget {
        deadline_seconds: 3600,
        stop_grace_seconds: 30,
        stdout_bytes: 262_144,
        stderr_bytes: 262_144,
        config_json_bytes: 65_536,
        private_state_bytes: 67_108_864,
        log_bytes: 268_435_456,
        memory_bytes: 2_147_483_648,
        pids_max: 64,
        cpu_max_percent: 200,
    }
}

/// 共享字节槽；「崩溃」＝丢弃会话后在同一份字节上重开。
#[derive(Clone, Default)]
pub struct Slot(Rc<RefCell<SlotInner>>);

#[derive(Default)]
pub struct SlotInner {
    pub bytes: Option<Vec<u8>>,
    pub countdown: Option<(u32, TransportError)>,
    pub owner: Option<(u32, u32)>,
}

impl Slot {
    pub fn bytes(&self) -> Option<Vec<u8>> {
        self.0.borrow().bytes.clone()
    }

    pub fn set_bytes(&self, bytes: Vec<u8>) {
        self.0.borrow_mut().bytes = Some(bytes);
    }

    /// 让第 `after + 1` 次发布失败，用来在任意阶段边界上切一刀。
    pub fn fail_replace_after(&self, after: u32, error: TransportError) {
        self.0.borrow_mut().countdown = Some((after, error));
    }

    pub fn fail_next_replace(&self, error: TransportError) {
        self.fail_replace_after(0, error);
    }

    pub fn state(&self) -> TransportState {
        state::decode(&self.bytes().expect("槽中已有权威记录")).expect("权威记录可解析")
    }

    pub fn publisher(&self) -> Box<dyn StatePublisher> {
        Box::new(SlotPublisher(self.clone()))
    }
}

pub struct SlotPublisher(Slot);

impl StatePublisher for SlotPublisher {
    fn read(&self) -> Result<Vec<u8>> {
        self.0.bytes().ok_or(TransportError::MissingStore)
    }

    fn replace(&mut self, bytes: &[u8]) -> Result<()> {
        // 发布失败必须让权威记录整体保持旧版本。
        let mut inner = self.0.0.borrow_mut();
        match inner.countdown {
            Some((0, error)) => {
                inner.countdown = None;
                return Err(error);
            }
            Some((remaining, error)) => inner.countdown = Some((remaining - 1, error)),
            None => {}
        }
        drop(inner);
        if bytes.len() > STATE_MAX_BYTES {
            return Err(TransportError::Budget);
        }
        self.0.set_bytes(bytes.to_vec());
        Ok(())
    }

    fn check(&self) -> Result<()> {
        Ok(())
    }

    fn owner_identity(&self) -> (u32, u32) {
        self.0.0.borrow().owner.unwrap_or((OWNER.uid, OWNER.gid))
    }
}

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
        _container: &ContainerIdentity,
    ) -> std::result::Result<bool, ContainmentGap> {
        self.observe_gap.map_or(Ok(self.running), Err)
    }

    fn close_spawn_gate(
        &mut self,
        _container: &ContainerIdentity,
    ) -> std::result::Result<(), ContainmentGap> {
        self.gate_closures += 1;
        Ok(())
    }

    fn read_events(
        &mut self,
        _container: &ContainerIdentity,
    ) -> std::result::Result<CgroupEvents, ContainmentGap> {
        Ok(CgroupEvents {
            populated: self.populated,
            frozen: false,
        })
    }

    fn drain(
        &mut self,
        _container: &ContainerIdentity,
    ) -> std::result::Result<ResourceCounts, ContainmentGap> {
        Ok(self.counts)
    }

    fn stop(
        &mut self,
        _container: &ContainerIdentity,
        _grace_seconds: u32,
    ) -> std::result::Result<(), ContainmentGap> {
        self.stops += 1;
        Ok(())
    }

    fn reap(
        &mut self,
        _container: &ContainerIdentity,
    ) -> std::result::Result<ExitObservation, ContainmentGap> {
        Ok(self.exit)
    }

    fn match_identity(
        &mut self,
        _container: &ContainerIdentity,
    ) -> std::result::Result<IdentityMatch, ContainmentGap> {
        Ok(self.identity)
    }
}

/// 一个已初始化的槽加一份假批准。
pub struct Fixture {
    pub approval: FakeApproval,
    pub slot: Slot,
}

impl Fixture {
    pub fn new() -> Self {
        let approval = FakeApproval::new();
        let runtime = PinnedRuntime::from_approval(&approval).expect("固定运行时可取");
        let slot = Slot::default();
        slot.set_bytes(
            state::encode(&initial_state(&approval, &runtime, OWNER).expect("首条记录可构造"))
                .expect("首条记录可编码"),
        );
        Self { approval, slot }
    }

    pub fn session(&self) -> RunSession<'_, FakeBackend> {
        RunSession::open(
            &self.approval,
            PinnedRuntime::from_approval(&self.approval).expect("固定运行时可取"),
            FakeBackend::new(),
            self.slot.publisher(),
        )
        .expect("既有槽可打开")
    }

    /// 走完 prepare → Intent ACK 的常规前置。
    pub fn prepared(&self, session: &mut RunSession<'_, FakeBackend>) -> PreparedRun {
        let prepared = session
            .prepare_once(1, RunOperation::RunOnce, 1_700_000_000_000)
            .expect("可以准备一次运行");
        acknowledge(session);
        prepared
    }
}

/// 确认当前待补审计事件。
pub fn acknowledge(session: &mut RunSession<'_, FakeBackend>) {
    let candidate = session
        .pending_audit()
        .expect("可以读待补事件")
        .expect("确有待补事件");
    let offset = next_offset(session);
    session
        .confirm_audit(&candidate, offset, [0xAB; 32])
        .expect("可以登记已核验回执");
}

fn next_offset(session: &RunSession<'_, FakeBackend>) -> u64 {
    session
        .load()
        .expect("可以读权威记录")
        .active
        .and_then(|record| {
            record
                .intent
                .receipt
                .map(|receipt| receipt.offset.get() + 1)
        })
        .unwrap_or(1)
}
