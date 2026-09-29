//! 双向 Bridge 驱动平面（T-132 P1，ADR-025 D1）：装进 `studiolo-human` 进程的新平面。
//! 一轮驱动 = InspectConfig → 守卫 → RunOnce（经 `SyncTransportOwner`）→ 静止 → 观察两侧 →
//! 规划（双向冲突先保全）→ 预留 → 逐动作执行 → 闭合 → 退休 → 保留期回收；顺序与失败即停点见
//! `docs/m2/sync-plane-design.md` 第 2 节。**默认关闭**：组合根没有 `--sync-transport`
//! 或私有配置缺 `sync_scope` 时本平面根本不被构造。
//! 写者边界：本模块及子模块从不持有 Broker 类型，canonical 侧读写全部经
//! `coordinator::sync_bridge` 的窄接缝；镜像只经 idmapped 视图根读写。
mod binding;
pub use binding::{bridge_binding, bridge_limits};

pub mod driver;
pub mod health;
pub mod job_backend;
pub mod mirror;
pub mod naming;
pub mod quiescence;
pub mod recovery;
pub mod witness;
use crate::CanonicalWorkbench;
pub use crate::sync_transport_config::{
    ConfigError as HeadlessConfigError, Expected as HeadlessExpected, FileType as HeadlessFileType,
    LocalConfigMatch as HeadlessConfigMatch, verify as verify_headless_config,
};
use mirror::stage::StageStore;
use recovery::PlaneStateStore;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::Duration,
};
use studiolo_audit_ledger::AuditLedger;
use studiolo_host_config::sync_transport::ApprovedTransport;
use studiolo_sync_bridge::sqlite::BridgeState;
use studiolo_sync_transport::owned_run::SyncTransportOwner;

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
    /// headless 已闭合但非干净退出；保留该次已确认的退出分类。
    #[error("headless 运行非干净结束")]
    Headless,
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
    /// 平面正在关停：本轮在最近的安全停点结束，不再发起新的运行、观察或执行。
    #[error("平面正在关停")]
    Stopping,
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
        // plane-state 或 stage 索引损坏均拒绝；stage 顺带回收孤儿。只核不留：`plane.lock` 是 flock
        // 排他租约，按打开文件描述计，同进程再开一次也会被自己拒绝（评审 P0-1）；每轮的
        // `RealPorts::open` 才是唯一持有者，平面不留第二个描述符。
        let state = PlaneStateStore::open(&inputs.plane_root).map_err(|_| PlaneError::Storage)?;
        if state.state().pending_purge.is_some() {
            return Err(PlaneError::Audit);
        }
        health.record_conflicts(state.state().conflict_stamps.len() as u64);
        let status = inputs
            .transport
            .inspect()
            .map_err(|_| PlaneError::Transport)?;
        if let Some(terminal) = status.last_terminal {
            let run = driver::HeadlessRun::from_terminal(terminal.run_id, terminal.observation);
            let forced = recovery::decide(
                state.state(),
                driver::approved_mode(inputs.approval.headless_mode()),
                health::now_unix_seconds(),
            )
            .forced_recovery;
            health.record_headless(&run, forced);
        }
        drop(state);
        drop(StageStore::open(&inputs.plane_root).map_err(|_| PlaneError::Storage)?);
        Ok(Arc::new(Self {
            approval: inputs.approval,
            transport: Mutex::new(inputs.transport),
            bridge: Mutex::new(inputs.bridge),
            workbench: inputs.workbench,
            audit: inputs.audit,
            plane_root: inputs.plane_root,
            health,
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

    /// 按固定间隔驱动，直到 `stop` 变真；每轮在阻塞线程里跑，轮内失败只记健康不退出。
    ///
    /// 轮次进行中收到停止信号：置关停标志，再等本轮在最近的安全停点结束（`driver::run_round_with`
    /// 的各个 `halt`），之后不再起新轮。最坏等待：headless 运行中＝一次 `request_stop(OwnerRequested)`
    /// 到闭合（审批 `stop_grace_seconds` 加回收与审计冲刷）；执行段中＝执行器跑完本批（不打断，
    /// 受静止票据到期约束）；其余步骤各自的一次 IO。
    ///
    /// # Errors
    /// 只有阻塞任务被中断才返回错误；驱动轮次自身的失败进健康证据。
    pub async fn drive_until(
        self: Arc<Self>,
        mut stop: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), String> {
        let stopping = Arc::new(AtomicBool::new(false));
        loop {
            tokio::select! {
                _ = stop.changed() => return Ok(()),
                () = tokio::time::sleep(ROUND_INTERVAL) => {}
            }
            if *stop.borrow() {
                return Ok(());
            }
            let plane = self.clone();
            let flag = stopping.clone();
            let mut round = tokio::task::spawn_blocking(move || {
                let now = health::now_unix_seconds();
                let outcome = driver::run_round(&plane, now, &flag);
                plane.health.record_round(now, &outcome);
            });
            let stopped = tokio::select! {
                joined = &mut round => {
                    joined.map_err(|_| "Bridge 驱动轮次中断".to_owned())?;
                    false
                }
                _ = stop.changed() => true,
            };
            if stopped {
                // 置关停标志，等本轮在最近的安全停点结束后返回，之后不再起新轮。
                stopping.store(true, Ordering::Release);
                round.await.map_err(|_| "Bridge 驱动轮次中断".to_owned())?;
                return Ok(());
            }
        }
    }
}
