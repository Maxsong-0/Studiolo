//! 每个会话的单向生命周期：创建、发布、关闭；两段清理都要确认完成。
use super::pool::Limits;
use rmcp::model::{RequestId, ServerJsonRpcMessage};
use rmcp::transport::streamable_http_server::session::SessionId;
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

struct Phase {
    initialized: bool,
    closing: bool,
    last_used: Instant,
}

pub(crate) struct Binding {
    pub(super) key: String,
    pub(crate) owner: String,
    pub(super) credential: String,
    pub(super) session: OnceLock<SessionId>,
    created: Instant,
    limits: Limits,
    phase: Mutex<Phase>,
    worker_closed: AtomicBool,
    worker_started: AtomicBool,
    creation_done: AtomicBool,
    creation_notice: tokio::sync::Notify,
    worker_done: tokio::sync::Notify,
    worker_stopping: AtomicBool,
    worker_stop: tokio::sync::Notify,
    cleanup_started: AtomicBool,
    initialize_id: OnceLock<RequestId>,
    initialize_answered: AtomicBool,
    work: Arc<tokio::sync::Semaphore>,
    pub(super) streams: super::streams::Streams,
}

impl Binding {
    pub(super) fn new(
        key: String,
        owner: &str,
        credential: &str,
        now: Instant,
        limits: Limits,
        work: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        Self {
            key,
            owner: owner.into(),
            credential: credential.into(),
            work,
            session: OnceLock::new(),
            created: now,
            limits,
            phase: Mutex::new(Phase {
                initialized: false,
                closing: false,
                last_used: now,
            }),
            worker_closed: AtomicBool::new(false),
            worker_done: tokio::sync::Notify::new(),
            worker_started: AtomicBool::new(false),
            creation_done: AtomicBool::new(false),
            creation_notice: tokio::sync::Notify::new(),
            worker_stopping: AtomicBool::new(false),
            worker_stop: tokio::sync::Notify::new(),
            cleanup_started: AtomicBool::new(false),
            initialize_id: OnceLock::new(),
            initialize_answered: AtomicBool::new(false),
            streams: super::streams::Streams::default(),
        }
    }

    /// SDK handler 独立于连接存活；工作许可必须随真正的供给 future 结束才释放。
    pub(crate) fn enter_work(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        self.work.clone().try_acquire_owned().ok()
    }

    pub(super) fn start_worker(&self) -> bool {
        let Ok(phase) = self.phase.lock() else {
            return false;
        };
        if phase.closing || self.expired(&phase, Instant::now()) {
            return false;
        }
        self.worker_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn creation_finished(&self) {
        self.creation_done.store(true, Ordering::Release);
        self.creation_notice.notify_one();
    }

    pub(super) async fn wait_creation(&self) {
        if self.worker_started.load(Ordering::Acquire) {
            while !self.creation_done.load(Ordering::Acquire) {
                self.creation_notice.notified().await;
            }
        }
    }

    pub(super) fn worker_finished(&self) {
        self.worker_closed.store(true, Ordering::Release);
        self.worker_done.notify_waiters();
    }

    /// 先记录应答 ID 再进入 SDK，关闭方才能唤醒未监听取消的初始化等待。
    pub(super) fn prepare_initialize(&self, id: RequestId) -> bool {
        self.phase.lock().is_ok_and(|phase| {
            !phase.closing && !phase.initialized && self.initialize_id.set(id).is_ok()
        })
    }

    pub(super) fn stop_worker(&self) {
        if let Ok(mut phase) = self.phase.lock() {
            phase.closing = true;
        }
        self.streams.close();
        self.worker_stopping.store(true, Ordering::Release);
        self.worker_stop.notify_waiters();
    }

    pub(super) fn is_stopping(&self) -> bool {
        self.worker_stopping.load(Ordering::Acquire)
    }

    pub(super) async fn wait_stop(&self) {
        loop {
            let notified = self.worker_stop.notified();
            if self.is_stopping() {
                return;
            }
            notified.await;
        }
    }

    pub(super) fn is_initialize_response(&self, message: &ServerJsonRpcMessage) -> bool {
        let id = match message {
            ServerJsonRpcMessage::Response(response) => Some(&response.id),
            ServerJsonRpcMessage::Error(error) => error.id.as_ref(),
            _ => None,
        };
        !self.initialize_answered.load(Ordering::Acquire)
            && id.is_some()
            && id == self.initialize_id.get()
    }

    pub(super) fn initialize_answered(&self) {
        self.initialize_answered.store(true, Ordering::Release);
    }

    pub(super) fn initialize_failure(&self) -> Option<ServerJsonRpcMessage> {
        let phase = self.phase.lock().ok()?;
        if !phase.closing || phase.initialized || self.initialize_answered.load(Ordering::Acquire) {
            return None;
        }
        Some(ServerJsonRpcMessage::error(
            rmcp::ErrorData::internal_error("MCP 初始化已关闭", None),
            Some(self.initialize_id.get()?.clone()),
        ))
    }

    pub(super) async fn wait_worker(&self) {
        loop {
            // Manager 和服务 close 可以同时等待；先创建 future 再检查，避免错过广播。
            let notified = self.worker_done.notified();
            if self.worker_closed.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    pub(crate) fn current(&self, store: &studiolo_supply::AgentStore) -> bool {
        store
            .credential_is_current(&self.owner, &self.credential)
            .unwrap_or(false)
    }

    fn expired(&self, phase: &Phase, now: Instant) -> bool {
        now.duration_since(self.created) >= self.limits.absolute
            || if phase.initialized {
                now.duration_since(phase.last_used) >= self.limits.idle
            } else {
                now.duration_since(self.created) >= self.limits.initialize
            }
    }

    pub(crate) fn live(&self, now: Instant) -> bool {
        self.phase
            .lock()
            .is_ok_and(|phase| phase.initialized && !phase.closing && !self.expired(&phase, now))
    }

    pub(crate) fn stale(&self, now: Instant) -> bool {
        self.phase
            .lock()
            .map_or(true, |phase| phase.closing || self.expired(&phase, now))
    }

    pub(super) fn initialize(&self, now: Instant) -> bool {
        let Ok(mut phase) = self.phase.lock() else {
            return false;
        };
        if phase.closing || phase.initialized || self.expired(&phase, now) {
            return false;
        }
        phase.initialized = true;
        phase.last_used = now;
        true
    }

    /// 只有成功且仍有效的内容供给续闲置期限；绝对期限不会改变。
    pub(crate) fn touch(&self, now: Instant) -> bool {
        let Ok(mut phase) = self.phase.lock() else {
            return false;
        };
        if !phase.initialized || phase.closing || self.expired(&phase, now) {
            return false;
        }
        phase.last_used = now;
        true
    }

    /// 关闭权只发一次，避免 DELETE、取消和清理并发时提前释放容量。
    pub(super) fn start_closing(&self) -> bool {
        self.stop_worker();
        self.cleanup_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}
