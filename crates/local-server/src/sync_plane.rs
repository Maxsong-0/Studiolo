//! 双向 Bridge 驱动平面（T-132 P1，ADR-025 D1）：装进 `studiolo-human` 进程的新平面。
//!
//! 一轮驱动 = InspectConfig → 守卫 → RunOnce（经 `SyncTransportOwner`）→ 静止 → 观察两侧 →
//! 规划 → 预留 → 逐动作执行 → 闭合 → 退休 → 保留期回收；顺序与失败即停点见
//! `docs/m2/sync-plane-design.md` 第 2 节。**默认关闭**：组合根没有 `--sync-transport`
//! 或私有配置缺 `sync_scope` 时本平面根本不被构造。
//!
//! 写者边界：本模块及子模块从不持有 Broker 类型，canonical 侧读写全部经
//! `coordinator::sync_bridge` 的窄接缝；镜像只经 idmapped 视图根读写。

pub mod driver;
pub mod health;
pub mod job_backend;
pub mod mirror;
pub mod naming;
pub mod quiescence;
pub mod recovery;
pub mod witness;

use mirror::stage::StageStore;
use recovery::PlaneStateStore;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use studiolo_audit_ledger::AuditLedger;
use studiolo_host_config::SyncScope;
use studiolo_host_config::sync_transport::ApprovedTransport;
use studiolo_sync_bridge::{Binding, Limits, sqlite::BridgeState};
use studiolo_sync_transport::owned_run::SyncTransportOwner;

use crate::CanonicalWorkbench;
pub use crate::sync_transport_config::{
    ConfigError as HeadlessConfigError, Expected as HeadlessExpected, FileType as HeadlessFileType,
    LocalConfigMatch as HeadlessConfigMatch, verify as verify_headless_config,
};

/// `<plane_root>` 下 Bridge 持久状态（sqlite 主库与租约）的固定子目录名；
/// 组合根与 `studiolo-maintain sync-init` 共用这一个常量。
pub const BRIDGE_DIR: &str = "bridge";

/// 两轮驱动之间的固定间隔；不可配置，避免把 headless 的 60 秒级同步排成风暴。
pub const ROUND_INTERVAL: Duration = Duration::from_secs(300);
/// 静止票据的最长使用窗口；超过就重新跑一次 RunOnce 取证据。
pub const QUIESCENT_WINDOW: Duration = Duration::from_secs(1800);

/// 平面的固定失败分类；不带路径、正文或凭据。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PlaneError {
    /// 审批文档、根或私有存储不满足装配前置。
    #[error("Bridge 平面配置或根审批失败")]
    Configuration,
    /// 受控 launcher 未就绪或传输运行槽拒绝。
    #[error("受控传输运行不可用")]
    Transport,
    /// headless 配置与审批值不一致（模式／排除项／文件类型）。
    #[error("headless 配置与审批不一致")]
    ConfigMismatch,
    /// 排除项守卫阻断：排除项清除前有镜像残留。
    #[error("排除项清除前有镜像残留")]
    ResidueBlocked,
    /// 处于退避或恢复窗口，本轮不跑。
    #[error("处于退避窗口")]
    Backoff,
    /// 静止证据缺失或已过期。
    #[error("没有可用的静止证据")]
    Quiescence,
    /// 任一侧观察不完整或范围未批准。
    #[error("观察不完整或范围未批准")]
    Observation,
    /// 删除熔断仍生效。
    #[error("删除熔断生效")]
    Fused,
    /// 有动作终态未知；本批不闭合。
    #[error("有动作终态未知")]
    Unknown,
    /// 审计账本不可用。
    #[error("审计账本不可用")]
    Audit,
    /// 私有存储（stage／plane-state／Bridge DB）不可用。
    #[error("Bridge 私有存储不可用")]
    Storage,
    /// canonical 写者或镜像视图暂不可用。
    #[error("canonical 写者或镜像视图暂不可用")]
    Unavailable,
}

/// 组合根交给平面的全部能力；每一项都已由可信启动层核验。
pub struct PlaneInputs {
    /// 已批准的传输能力；生命期与进程相同，由组合根 `Box::leak` 得到。
    pub approval: &'static ApprovedTransport<'static>,
    /// 已打开的传输运行槽。
    pub transport: SyncTransportOwner<'static>,
    /// 已打开并核验绑定的 Bridge 持久状态。
    pub bridge: BridgeState,
    /// canonical 写者句柄；平面只经 `coordinator::sync_bridge` 接缝使用它。
    pub workbench: Arc<CanonicalWorkbench>,
    /// 权威审计账本。
    pub audit: Arc<dyn AuditLedger>,
    /// 平面私有根（外 stage、plane-state.json、plane.lock 都在这里）。
    pub plane_root: PathBuf,
}

/// 已装配的驱动平面；一个进程至多一个。
pub struct SyncPlane {
    pub(crate) approval: &'static ApprovedTransport<'static>,
    pub(crate) transport: Mutex<SyncTransportOwner<'static>>,
    pub(crate) bridge: Mutex<BridgeState>,
    pub(crate) workbench: Arc<CanonicalWorkbench>,
    pub(crate) audit: Arc<dyn AuditLedger>,
    pub(crate) plane_root: PathBuf,
    pub(crate) health: Arc<health::PlaneHealth>,
    pub(crate) state: Mutex<PlaneStateStore>,
    pub(crate) stage: Mutex<StageStore>,
}

impl SyncPlane {
    /// 打开平面：核对 plane-state、外 stage 索引与残留；任一失败即拒绝装配。
    ///
    /// # Errors
    /// 私有存储损坏或不可用返回 `Storage`；审批复验失败返回 `Configuration`。
    pub fn open(inputs: PlaneInputs) -> Result<Arc<Self>, PlaneError> {
        inputs
            .approval
            .revalidate()
            .map_err(|_| PlaneError::Configuration)?;
        let health = Arc::new(health::PlaneHealth::new(inputs.approval.manual_quota()));
        // 私有存储失败关闭：plane-state.json 撕裂／校验和不符、外 stage 索引损坏都不装配；
        // stage 打开时顺带回收孤儿对象。两者都过才有平面。
        let state = PlaneStateStore::open(&inputs.plane_root).map_err(|_| PlaneError::Storage)?;
        let stage = StageStore::open(&inputs.plane_root).map_err(|_| PlaneError::Storage)?;
        Ok(Arc::new(Self {
            approval: inputs.approval,
            transport: Mutex::new(inputs.transport),
            bridge: Mutex::new(inputs.bridge),
            workbench: inputs.workbench,
            audit: inputs.audit,
            plane_root: inputs.plane_root,
            health,
            state: Mutex::new(state),
            stage: Mutex::new(stage),
        }))
    }

    /// 健康证据来源；`startup/health_input.rs` 用它接第六节与 owner 活性。
    #[must_use]
    pub fn health_probe(&self) -> Arc<dyn health::BridgeHealthProbe> {
        self.health.clone()
    }

    /// 已批准的传输能力（仅供本平面子模块）。
    #[must_use]
    pub fn approval(&self) -> &'static ApprovedTransport<'static> {
        self.approval
    }

    /// 传输运行槽；一轮内串行持锁。
    #[must_use]
    pub fn transport(&self) -> &Mutex<SyncTransportOwner<'static>> {
        &self.transport
    }

    /// Bridge 持久状态；一轮内串行持锁。
    #[must_use]
    pub fn bridge(&self) -> &Mutex<BridgeState> {
        &self.bridge
    }

    /// canonical 写者句柄；只经 `coordinator::sync_bridge` 接缝使用。
    #[must_use]
    pub fn workbench(&self) -> &Arc<CanonicalWorkbench> {
        &self.workbench
    }

    /// 权威审计账本。
    #[must_use]
    pub fn audit(&self) -> &Arc<dyn AuditLedger> {
        &self.audit
    }

    /// 平面私有根。
    #[must_use]
    pub fn plane_root(&self) -> &std::path::Path {
        &self.plane_root
    }

    /// 恢复协议与排除项守卫的持久小状态；一轮内串行持锁。
    #[must_use]
    pub fn state(&self) -> &Mutex<PlaneStateStore> {
        &self.state
    }

    /// 删除前保全的外 stage；一轮内串行持锁。
    #[must_use]
    pub fn stage(&self) -> &Mutex<StageStore> {
        &self.stage
    }

    /// 按固定间隔驱动，直到 `stop` 变真；每轮在阻塞线程里跑，轮内失败只记健康不退出。
    ///
    /// # Errors
    /// 只有阻塞任务被中断才返回错误；驱动轮次自身的失败进健康证据。
    pub async fn drive_until(
        self: Arc<Self>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), String> {
        loop {
            tokio::select! {
                _ = stop.changed() => return Ok(()),
                () = tokio::time::sleep(ROUND_INTERVAL) => {}
            }
            if *stop.borrow() {
                return Ok(());
            }
            let plane = self.clone();
            tokio::task::spawn_blocking(move || {
                let now = health::now_unix_seconds();
                let outcome = driver::run_round(&plane, now);
                plane.health.record_round(now, &outcome);
            })
            .await
            .map_err(|_| "Bridge 驱动轮次中断".to_owned())?;
        }
    }
}

/// Bridge 持久状态的绑定：链身份、Vault 身份与两个根 token 全部由审批与已核验根身份派生。
///
/// `canonical_identity` 是可信启动层（`ServingRoots`）或维护入口（`CanonicalServingRoots`）
/// 核验过的 canonical 根 `(dev, ino)`。组合根与 `studiolo-maintain sync-init` 必须算出同一个
/// 绑定，否则 `BridgeState::open_existing` 按绑定不符拒绝。
#[must_use]
pub fn bridge_binding(approval: &ApprovedTransport<'_>, canonical_identity: (u64, u64)) -> Binding {
    let scope = approval.binding();
    let roots = approval.roots();
    Binding {
        link_id: scope.link_id,
        canonical_vault: vault_identity(&scope.vault_uuid.to_string()),
        remote_vault: scope.remote_vault,
        canonical_root_token: root_token("canonical", canonical_identity),
        mirror_root_token: root_token("mirror", roots.mirror_identity()),
        scope_hash: scope.scope_hash,
        scope_epoch: scope.scope_epoch,
    }
}

/// Bridge 执行器的保守预算：路径与输出上限固定，单对象字节上限取范围内最大的 `max_bytes`。
#[must_use]
pub fn bridge_limits(scope: &SyncScope) -> Limits {
    Limits::conservative(scope.max_bytes())
}

/// 根 token：域分隔 sha256(角色 ‖ dev ‖ ino)。
fn root_token(role: &str, identity: (u64, u64)) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-plane.root.v1\0");
    hash.update(role.as_bytes());
    hash.update(identity.0.to_be_bytes());
    hash.update(identity.1.to_be_bytes());
    hash.finalize().into()
}

/// Vault 身份的 16 字节投影：域分隔 sha256 的前 16 字节。
fn vault_identity(uuid: &str) -> [u8; 16] {
    let mut hash = Sha256::new();
    hash.update(b"studiolo.sync-plane.vault.v1\0");
    hash.update(uuid.as_bytes());
    let digest: [u8; 32] = hash.finalize().into();
    let mut identity = [0u8; 16];
    identity.copy_from_slice(&digest[..16]);
    identity
}
