//! 真实 snapshot 后台加载入口的私有同步点；客户端取消不应释放实际作业许可。

use std::{fs, sync::Mutex};

use super::*;

#[derive(Default)]
pub(crate) struct Probe {
    gate: Mutex<Option<Gate>>,
}

struct Gate {
    started: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Probe {
    pub(crate) fn enter(&self) {
        let gate = self.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.started.send(());
            let _ = gate.release.recv();
        }
    }
}

#[tokio::test]
async fn cancelled_snapshot_keeps_its_actual_blocking_refresh_slot() {
    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("vault");
    let assets_path = temp.path().join("assets");
    fs::create_dir(&vault_path).unwrap();
    fs::create_dir(&assets_path).unwrap();
    fs::write(vault_path.join("note.md"), "# 虚构正文\n").unwrap();
    let vault = ReadOnlyVault::open(&vault_path).unwrap();
    let generation = load(&vault, 1).unwrap();
    let (started, entered) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let state = AppState {
        source: Source::Replica(Arc::new(vault)),
        assets: Arc::new(ReadOnlyVault::open(&assets_path).unwrap()),
        current: ViewStore::replica(generation),
        refresh: Arc::new(tokio::sync::Mutex::new(())),
        queries: Arc::new(tokio::sync::Semaphore::new(2)),
        source_label: "虚构库".into(),
        authority: "127.0.0.1:4317".into(),
        origin: "http://127.0.0.1:4317".into(),
        access: None,
        sessions: None,
        canonical: None,
        refresh_probe: Arc::new(Probe {
            gate: Mutex::new(Some(Gate {
                started,
                release: wait_release,
            })),
        }),
    };
    let request_state = state.clone();
    let request = tokio::spawn(async move {
        snapshot(State(request_state), Query(SnapshotQuery { refresh: true })).await
    });
    entered.await.unwrap();
    request.abort();
    assert!(matches!(request.await, Err(error) if error.is_cancelled()));

    assert!(
        state.refresh.try_lock().is_err(),
        "HTTP 取消释放了尚未结束的真实刷新槽"
    );
    release.send(()).unwrap();
    drop(state.refresh.lock().await);
}
