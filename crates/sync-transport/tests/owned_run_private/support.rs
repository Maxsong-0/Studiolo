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
use std::{
    num::NonZeroU64,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

/// 可跨线程的可变槽；接口沿用 `RefCell` 的 `borrow`／`borrow_mut`，用例不用改写。
pub struct Shared<T>(Mutex<T>);

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn borrow(&self) -> MutexGuard<'_, T> {
        self.0.lock().expect("测试槽未中毒")
    }

    pub fn borrow_mut(&self) -> MutexGuard<'_, T> {
        self.0.lock().expect("测试槽未中毒")
    }
}

impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
use studiolo_core::VaultUuid;

mod backend;
pub use backend::*;

pub const PRIVATE_ROOT: &str = "/srv/studiolo/transport";
pub const MIRROR_ROOT: &str = "/srv/studiolo/mirror";
pub const MIRROR_VIEW_ROOT: &str = "/srv/studiolo/mirror-view";
pub const SERVICE_UNIT: &str = "studiolo-sync-launcher@link-1.service";
pub const CGROUP_PREFIX: &str = "/sys/fs/cgroup/studiolo.slice/payload";
/// 假批准里的专用 payload 身份；假 launcher 的 `probe` 报回同一值。
pub const PAYLOAD_UID: u32 = 61001;
pub const PAYLOAD_GID: u32 = 61001;
pub const OWNER: OwnerIdentity = OwnerIdentity {
    uid: 1000,
    gid: 1000,
};

/// 假批准：字段可改，用来模拟配置漂移与根替换。
pub struct FakeApproval {
    pub fresh: Shared<bool>,
    pub binding: Shared<TransportBindingFacts>,
    pub roots: Shared<RootPins>,
    pub runtime: RuntimePins,
    pub backend: Shared<BackendDescriptor>,
    pub budget: Shared<RunBudget>,
}

impl FakeApproval {
    pub fn new() -> Self {
        Self {
            fresh: Shared::new(true),
            binding: Shared::new(TransportBindingFacts {
                vault_uuid: VaultUuid::new(),
                link_id: [1u8; 16],
                remote_vault: [2u8; 16],
                scope_epoch: NonZeroU64::new(1).expect("范围代次非零"),
                scope_hash: [3u8; 32],
                approved_config_digest: [4u8; 32],
            }),
            roots: Shared::new(RootPins {
                private_root: PathBuf::from(PRIVATE_ROOT),
                private_identity: (64, 11),
                mirror_root: PathBuf::from(MIRROR_ROOT),
                mirror_identity: (64, 12),
                mirror_view_root: PathBuf::from(MIRROR_VIEW_ROOT),
            }),
            runtime: RuntimePins {
                version: PINNED_RUNTIME_VERSION.to_owned(),
                node_path: PathBuf::from("/opt/studiolo/node/bin/node"),
                cli_path: PathBuf::from("/opt/studiolo/obsidian-headless/cli.js"),
                closure_digest: [5u8; 32],
            },
            backend: Shared::new(BackendDescriptor {
                service_unit: SERVICE_UNIT.to_owned(),
                payload_uid: PAYLOAD_UID,
                payload_gid: PAYLOAD_GID,
                payload_cgroup_prefix: CGROUP_PREFIX.to_owned(),
                digest: [6u8; 32],
            }),
            budget: Shared::new(budget()),
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
pub struct Slot(Arc<Shared<SlotInner>>);

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
    pub fn prepared<B: ContainmentBackend>(&self, session: &mut RunSession<'_, B>) -> PreparedRun {
        let prepared = session
            .prepare_once(1, RunOperation::RunOnce, 1_700_000_000_000)
            .expect("可以准备一次运行");
        acknowledge(session);
        prepared
    }
}

/// 确认当前待补审计事件；对任何后端都一样，假 launcher 驱动的真实 adapter 也用它。
pub fn acknowledge<B: ContainmentBackend>(session: &mut RunSession<'_, B>) {
    let candidate = session
        .pending_audit()
        .expect("可以读待补事件")
        .expect("确有待补事件");
    let offset = next_offset(session);
    session
        .confirm_audit(&candidate, offset, [0xAB; 32])
        .expect("可以登记已核验回执");
}

fn next_offset<B: ContainmentBackend>(session: &RunSession<'_, B>) -> u64 {
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
