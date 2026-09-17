//! 私有真实作业同步点；文件系统夹具留在 tests 目录，不增加生产文件权限。

mod fixture;
mod lifecycle;
mod publication;
mod staging;
mod staging_notify;
mod staging_rejections;
mod watcher;

use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

pub(super) const TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Probe {
    gates: Mutex<BTreeMap<&'static str, Gate>>,
    pub(crate) os_events: AtomicBool,
    pub(super) captures: AtomicUsize,
    read_observer: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

struct Gate {
    entered: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            gates: Mutex::new(BTreeMap::new()),
            os_events: AtomicBool::new(true),
            captures: AtomicUsize::new(0),
            read_observer: Mutex::new(None),
        }
    }
}

impl Probe {
    pub(super) fn arm(
        &self,
        point: &'static str,
    ) -> (
        tokio::sync::oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (entered, observed) = tokio::sync::oneshot::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        self.gates.lock().unwrap().insert(
            point,
            Gate {
                entered,
                release: wait_release,
            },
        );
        (observed, release)
    }

    fn enter(&self, point: &'static str) {
        let gate = self.gates.lock().unwrap().remove(point);
        if let Some(gate) = gate {
            let _ = gate.entered.send(());
            let _ = gate.release.recv_timeout(TIMEOUT);
        }
    }

    pub(crate) fn capture(&self) {
        self.captures.fetch_add(1, Ordering::SeqCst);
        self.enter("capture");
    }
    pub(crate) fn building(&self) {
        self.enter("building");
    }
    pub(crate) fn built(&self) {
        self.enter("built");
    }
    pub(crate) fn ignored_read(&self) {
        if let Some(observer) = self.read_observer.lock().unwrap().take() {
            let _ = observer.send(());
        }
    }
    #[cfg(target_os = "linux")]
    pub(super) fn observe_read(&self) -> tokio::sync::oneshot::Receiver<()> {
        let (send, receive) = tokio::sync::oneshot::channel();
        *self.read_observer.lock().unwrap() = Some(send);
        receive
    }
}
