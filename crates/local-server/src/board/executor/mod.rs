//! Trusted production Runner composition; omitted configuration leaves it unavailable.
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
/// Safe read-only projection of a controlled execution; no prompt, response or credential.
#[derive(Clone, Debug, Serialize)]
pub struct RunSummary {
    /// Owning task.
    pub task_id: TaskId,
    /// Attempt identity.
    pub attempt_id: AttemptId,
    /// Pinned adapter kind.
    pub adapter: String,
    /// Lifecycle projection: running, candidate_review, candidate_retained, stopped or failed.
    pub state: String,
    /// Observed monotonic counters.
    pub progress: RunnerProgress,
}
/// Authenticated caller chooses a work item and initiating identity, never execution paths/keys.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    /// Explicit work item to run.
    pub task_id: TaskId,
    /// Registered initiating AI type, reauthorized by the plane.
    pub agent_type: String,
    /// Registered initiating instance, including ancestor revocation checks.
    pub instance_id: String,
}
/// Default-frozen Runner/Gateway plane, held by the main application.
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
    // Unknown logical tools are not silently introduced through a deployment string.
    let allowed = values.any(|tool| tool == "model.relay");
    studiolo_core::PermissionSet::new(if allowed {
        vec![studiolo_core::ToolId::from_static("model.relay")]
    } else {
        vec![]
    })
}
impl RunnerPlane {
    /// Opens the plane without launching a process, reading a model key or permitting egress.
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
    /// Starts a single bounded Unix ingress loop; call once after successful open.
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
    /// Reads the current admission gate without rerunning deployment probes.
    #[must_use]
    pub fn is_frozen(&self) -> bool {
        self.authority.frozen.load(Ordering::SeqCst)
    }
    /// Freezes immediately; unfreeze requires all configured runtime profiles to pass live probes.
    pub fn freeze(&self, frozen: bool) -> Result<(), String> {
        if frozen {
            self.admission.invalidate();
            self.authority.frozen.store(true, Ordering::SeqCst);
            // Wait for a previously admitted start to finish before acknowledging freeze.
            let _admission = self.admission.lock()?;
            // Also invalidate probes which entered while an older start held the barrier.
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
    /// Safe status projection for authenticated owner views.
    pub fn replay(&self) -> Result<Vec<RunSummary>, String> {
        Ok(self
            .runs
            .lock()
            .map_err(|_| "Runner状态不可用")?
            .values()
            .map(|r| r.summary.clone())
            .collect())
    }
    /// Revokes admissions and isolates all current process scopes before host shutdown.
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
        // PreparedServer error paths can drop this plane before serve/shutdown is reached.
        // Also stop a serving thread so its relay/Board/Writer ownership cannot outlive us.
        let _ = self.shutdown();
    }
}

#[cfg(test)]
#[path = "landing_tests.rs"]
mod landing_tests;
