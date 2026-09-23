//! 受信任的生产 `Runner` 组装；未提供配置时它不可用。
mod admission;
mod authority;
mod cleanup;
mod config;
mod landing;
mod landing_candidate;
mod landing_store;
mod lifecycle;
pub use landing::{CodeLandingResult, CodeLandingReviewGuard};
mod prepare;
mod publish;
mod recovery;
mod roots;
mod socket;
mod start;
pub use config::{PolicyConfig, ProjectConfig, ProviderConfig, RunnerConfig, SourceConfig};
pub use recovery::isolate_persisted;
pub use roots::validate_isolated_roots;

use crate::board::BoardPlane;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use studiolo_core::{AttemptId, TaskId, VaultPath};
use studiolo_gateway::{
    DurableRelay, PermissionLayers, RelayGrant, RelayServices, RemoteRelayTransport,
};
use studiolo_runner_adapter::{
    AdapterAvailability, RealRunner, RunHandle, RunSpec, RunnerAdapter, RunnerProgress,
};

#[derive(Clone)]
struct Binding {
    agent_type: String,
    instance_id: String,
    project: VaultPath,
    grant: Option<RelayGrant>,
}
struct ActiveRun {
    runner: RealRunner,
    handle: RunHandle,
    spec: RunSpec,
    summary: RunSummary,
    ticket: PathBuf,
    cleanup_pending: bool,
    launch_failed: bool,
}
/// 受控执行的安全只读投影；不含提示词、响应或凭据。
#[derive(Clone, Debug, Serialize)]
pub struct RunSummary {
    /// 所属任务。
    pub task_id: TaskId,
    /// 尝试身份。
    pub attempt_id: AttemptId,
    /// 钉死的适配器类型。
    pub adapter: String,
    /// 生命周期投影：`running`、`candidate_review`、`candidate_retained`、`stopped` 或 `failed`。
    pub state: String,
    /// 观察到的单调计数器。
    pub progress: RunnerProgress,
}
/// 经认证的调用方只选择工作项与发起身份，绝不指定执行路径或密钥。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    /// 要运行的显式工作项。
    pub task_id: TaskId,
    /// 已登记的发起方 `AI` 类型，由平面重新授权。
    pub agent_type: String,
    /// 已登记的发起实例，含祖先撤销检查。
    pub instance_id: String,
}
/// 默认冻结的 `Runner`／`Gateway` 平面，由主应用持有。
pub struct RunnerPlane {
    authority: Arc<authority::Authority>,
    relay: Arc<DurableRelay>,
    runs: Mutex<BTreeMap<AttemptId, ActiveRun>>,
    socket: socket::SocketLease,
    admission: admission::Admission,
    listener: Mutex<Option<std::os::unix::net::UnixListener>>,
    stop: Arc<AtomicBool>,
    ingress_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
pub(super) fn hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn permissions(mut values: impl Iterator<Item = String>) -> studiolo_core::PermissionSet {
    // 未知的逻辑工具不会借部署字符串被悄悄引入。
    let allowed = values.any(|tool| tool == "model.relay");
    studiolo_core::PermissionSet::new(if allowed {
        vec![studiolo_core::ToolId::from_static("model.relay")]
    } else {
        vec![]
    })
}
impl RunnerPlane {
    /// 打开平面，但不启动进程、不读取模型密钥，也不放行外发。
    pub fn open(board: Arc<BoardPlane>, config: RunnerConfig) -> Result<Arc<Self>, String> {
        {
            let engine = board.engine.lock().map_err(|_| "黑板不可用")?;
            let roots = &engine.broker().workbench.core.roots;
            let mut forbidden = board.isolated_roots.clone();
            forbidden.extend([
                roots.vault_directory().to_path_buf(),
                roots.writer_data_directory().to_path_buf(),
                board
                    .storage
                    .file()
                    .parent()
                    .ok_or("黑板控制根缺失")?
                    .to_path_buf(),
            ]);
            validate_isolated_roots(&config, &forbidden)?;
        }
        cleanup::startup(&board, &config)?;
        let (listener, socket) =
            socket::SocketLease::bind(config.state_root.join("runner-relay.sock"))?;
        let layers = PermissionLayers::new(
            permissions(config.deployment_tools.clone().into_iter()),
            permissions(config.policies.iter().flat_map(|p| p.policy_tools.clone())),
            permissions(config.policies.iter().flat_map(|p| p.role_tools.clone())),
        );
        let authority = Arc::new(authority::Authority {
            board: board.clone(),
            config: config.clone(),
            frozen: AtomicBool::new(true),
            bindings: Mutex::new(BTreeMap::new()),
        });
        let relay = DurableRelay::open(
            &config.state_root.join("relay.sqlite3"),
            config.raw_root.clone(),
            RelayServices {
                guard: authority.clone(),
                policy: authority.clone(),
                clock: Arc::new(authority::SystemClock),
                audit: board.audit.clone(),
                permissions: layers,
                transport: Arc::new(RemoteRelayTransport::new(
                    config.upstream_socket.clone(),
                    config.upstream_uid,
                )),
                publisher: Arc::new(publish::Publisher(authority.clone())),
            },
        )
        .map_err(|_| "Runner Gateway持久状态不可用")?;
        Ok(Arc::new(Self {
            authority,
            relay: Arc::new(relay),
            runs: Mutex::new(BTreeMap::new()),
            socket,
            admission: admission::Admission::default(),
            listener: Mutex::new(Some(listener)),
            stop: Arc::new(AtomicBool::new(false)),
            ingress_thread: Mutex::new(None),
        }))
    }
    /// 启动唯一一个有界的 `Unix` 入口循环；成功打开之后调用一次。
    pub fn serve(self: &Arc<Self>) -> Result<(), String> {
        let _admission = self.admission.lock()?;
        if self.stop.load(Ordering::SeqCst) {
            return Err("Runner已关闭".into());
        }
        let listener = self
            .listener
            .lock()
            .map_err(|_| "Runner监听不可用")?
            .take()
            .ok_or("Runner监听已启动")?;
        let stop = self.stop.clone();
        let relay = self.relay.clone();
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = studiolo_gateway::serve_relay_connection(&relay, stream);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(50))
                    }
                    Err(_) => break,
                }
            }
        });
        *self.ingress_thread.lock().map_err(|_| "Runner监听不可用")? = Some(thread);
        Ok(())
    }
    /// 读取当前准入闸门，不重跑部署探针。
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.authority.frozen.load(Ordering::SeqCst)
    }
    /// 冻结立即生效；解冻要求所有已配置的运行时配置档都通过实时探针。
    pub fn freeze(&self, frozen: bool) -> Result<(), String> {
        if frozen {
            self.admission.invalidate();
            self.authority.frozen.store(true, Ordering::SeqCst);
            // 确认冻结之前，先等此前已获准入的启动完成。
            let _admission = self.admission.lock()?;
            // 同时作废在旧启动持有屏障期间进入的探针。
            self.admission.invalidate();
            self.authority.frozen.store(true, Ordering::SeqCst);
            return self
                .relay
                .set_frozen(true)
                .map_err(|_| "Gateway冻结失败".into());
        }
        let epoch = self.admission.probe_epoch()?;
        for provider in &self.authority.config.providers {
            let runner = self.probe_runner(provider)?;
            if !matches!(
                runner.probe().map_err(|_| "Runner probe失败")?,
                AdapterAvailability::Ready
            ) {
                return Err("真实Runner隔离探针尚未通过".into());
            }
        }
        RemoteRelayTransport::new(
            self.authority.config.upstream_socket.clone(),
            self.authority.config.upstream_uid,
        )
        .probe()
        .map_err(|_| "独立Gateway身份或socket不可用")?;
        self.admission.unfreeze(
            epoch,
            || {
                self.relay
                    .set_frozen(false)
                    .map_err(|_| "Gateway解冻失败")?;
                self.authority.frozen.store(false, Ordering::SeqCst);
                Ok(())
            },
            || {
                self.authority.frozen.store(true, Ordering::SeqCst);
                self.relay
                    .set_frozen(true)
                    .map_err(|_| "Gateway冻结失败".into())
            },
        )
    }
    /// 供经认证的主人视图使用的安全状态投影。
    pub fn replay(&self) -> Result<Vec<RunSummary>, String> {
        Ok(self
            .runs
            .lock()
            .map_err(|_| "Runner状态不可用")?
            .values()
            .map(|r| r.summary.clone())
            .collect())
    }
    /// 宿主机关停之前撤销准入，并隔离当前全部进程 `scope`。
    pub fn shutdown(&self) -> Result<(), String> {
        self.admission.close();
        self.stop.store(true, Ordering::SeqCst);
        let mut result = self.freeze(true);
        {
            let mut runs = self.runs.lock().map_err(|_| "Runner状态不可用")?;
            let now = jiff::Timestamp::now().as_millisecond();
            for run in runs.values_mut() {
                let cleanup = self.stop_and_close(run, now);
                run.cleanup_pending = cleanup.is_err();
                result = result.and(cleanup);
            }
        }
        if let Some(thread) = self
            .ingress_thread
            .lock()
            .map_err(|_| "Runner监听不可用")?
            .take()
        {
            result = result.and(thread.join().map_err(|_| "Runner监听收尾失败".into()));
        }
        self.listener.lock().map_err(|_| "Runner监听不可用")?.take();
        result = result.and(self.socket.cleanup());
        result
    }
}

impl Drop for RunnerPlane {
    fn drop(&mut self) {
        // `PreparedServer` 的错误路径可能在走到 `serve`／`shutdown` 之前就析构本平面。
        // 同时停掉服务线程，使它对 `relay`／`Board`／`Writer` 的所有权不会比本平面活得更久。
        let _ = self.shutdown();
    }
}

#[cfg(test)]
#[path = "landing_tests.rs"]
mod landing_tests;
