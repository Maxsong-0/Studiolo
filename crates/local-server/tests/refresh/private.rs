//! 只读副本刷新的合流、失败与恢复契约：派生次数一律来自真实调用计数，不靠计时。
//!
//! 这里直接驱动 `snapshot` 解析查询串之后的主体 `refreshed_overview`，因此断言的是真实的
//! HTTP 入口行为：受理、合流、结算、答复全在被测代码里发生，测试只提供一个记录调用次数
//! 并可按需卡住的探针。

use std::{
    fs,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use tempfile::TempDir;

use super::*;

/// 派生探针：记录受理数与真实派生次数，并可把第一次派生卡在原地或让它死亡。
pub(crate) struct Probe {
    admissions: tokio::sync::watch::Sender<usize>,
    derivations: AtomicUsize,
    gate: Mutex<Option<Gate>>,
    dies_after_release: AtomicBool,
}

struct Gate {
    started: tokio::sync::oneshot::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

impl Default for Probe {
    fn default() -> Self {
        Self {
            admissions: tokio::sync::watch::channel(0).0,
            derivations: AtomicUsize::new(0),
            gate: Mutex::new(None),
            dies_after_release: AtomicBool::new(false),
        }
    }
}

impl Probe {
    /// 每次刷新请求被闸门受理时调用一次，含被合流掉的请求。
    pub(crate) fn admitted(&self) {
        self.admissions.send_modify(|count| *count += 1);
    }

    /// 每次真实派生开始时调用一次；装了闸就卡住，注入了死亡就 panic。
    pub(crate) fn enter(&self) {
        self.derivations.fetch_add(1, Ordering::SeqCst);
        let gate = self
            .gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(gate) = gate {
            let _ = gate.started.send(());
            let _ = gate.release.recv();
        }
        if self.dies_after_release.swap(false, Ordering::SeqCst) {
            panic!("测试注入的刷新运行者死亡");
        }
    }

    fn derivations(&self) -> usize {
        self.derivations.load(Ordering::SeqCst)
    }

    /// 等到受理数达到目标；用状态变更通知而不是睡眠或轮询。
    async fn admissions_reach(&self, target: usize) {
        let mut receiver = self.admissions.subscribe();
        let _ = receiver.wait_for(|count| *count >= target).await;
    }
}

/// 卡住第一次派生的遥控器：`entered` 等它真的开始，`release` 放它继续。
struct Holder {
    entered: tokio::sync::oneshot::Receiver<()>,
    release: std::sync::mpsc::Sender<()>,
}

fn probe_holding_first_derivation() -> (Arc<Probe>, Holder) {
    let (started, entered) = tokio::sync::oneshot::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let probe = Probe {
        gate: Mutex::new(Some(Gate {
            started,
            release: wait_release,
        })),
        ..Probe::default()
    };
    (Arc::new(probe), Holder { entered, release })
}

fn fixture() -> (TempDir, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let vault_path = temp.path().join("vault");
    let assets_path = temp.path().join("assets");
    fs::create_dir(&vault_path).unwrap();
    fs::create_dir(&assets_path).unwrap();
    fs::write(vault_path.join("note.md"), "# 虚构正文\n").unwrap();
    let vault = ReadOnlyVault::open(&vault_path).unwrap();
    let generation = load(&vault, 1).unwrap();
    let state = AppState {
        source: Source::Replica(Arc::new(vault)),
        assets: Arc::new(ReadOnlyVault::open(&assets_path).unwrap()),
        current: ViewStore::replica(generation),
        refresh: Arc::new(crate::refresh::RefreshCoalescer::new()),
        queries: Arc::new(tokio::sync::Semaphore::new(2)),
        source_label: "虚构库".into(),
        authority: "127.0.0.1:4317".into(),
        origin: "http://127.0.0.1:4317".into(),
        access: None,
        sessions: None,
        canonical: None,
        refresh_probe: Arc::new(Probe::default()),
    };
    (temp, state)
}

fn with_probe(probe: Arc<Probe>) -> (TempDir, AppState) {
    let (temp, mut state) = fixture();
    state.refresh_probe = probe;
    (temp, state)
}

fn vault_root(temp: &TempDir) -> std::path::PathBuf {
    temp.path().join("vault")
}

fn refresh_request(state: &AppState) -> tokio::task::JoinHandle<Result<(), ApiError>> {
    let state = state.clone();
    // 查询串已解析为 `refresh=true` 之后的 `snapshot` 主体：受理、合流、结算与答复全在这里。
    tokio::spawn(async move { refreshed_overview(state, true).await.map(|_| ()) })
}

/// 只用于把契约回归变成失败而不是永久挂起；没有任何断言依赖这个时长。
async fn settled(request: tokio::task::JoinHandle<Result<(), ApiError>>) -> Result<(), ApiError> {
    tokio::time::timeout(Duration::from_secs(30), request)
        .await
        .expect("刷新请求在合理时间内得到答复，而不是永久等待")
        .expect("刷新请求任务本身不 panic")
}

/// 等到答复并断言刷新成功；`ApiError` 不实现 `Debug`，所以自己打印状态与详情。
async fn completed(request: tokio::task::JoinHandle<Result<(), ApiError>>) {
    if let Err(ApiError(status, message)) = settled(request).await {
        panic!("刷新应当成功，实际是 {status}：{message}");
    }
}

/// 等到答复并取出失败；答复成功即为契约被破坏。
async fn failed(request: tokio::task::JoinHandle<Result<(), ApiError>>) -> (StatusCode, String) {
    match settled(request).await {
        Ok(()) => panic!("刷新应当失败，实际被当成成功"),
        Err(ApiError(status, message)) => (status, message),
    }
}

async fn generation_of(state: &AppState) -> u64 {
    state.current.snapshot().unwrap().generation.number
}

#[tokio::test]
async fn cancelled_snapshot_keeps_its_actual_blocking_refresh_slot() {
    let (probe, holder) = probe_holding_first_derivation();
    let (temp, state) = with_probe(Arc::clone(&probe));
    let request = refresh_request(&state);
    holder.entered.await.unwrap();
    request.abort();
    assert!(matches!(request.await, Err(error) if error.is_cancelled()));

    assert!(
        state.refresh.is_busy(),
        "HTTP 取消释放了尚未结束的真实刷新槽"
    );
    holder.release.send(()).unwrap();

    // 取消之后仍由派生自己结算并发布，下一次请求接着往前推进代次。
    completed(refresh_request(&state)).await;
    assert_eq!(generation_of(&state).await, 3);
    assert_eq!(probe.derivations(), 2);
    drop(temp);
}

#[tokio::test]
async fn nine_concurrent_refresh_requests_run_the_derivation_twice_instead_of_nine_times() {
    let (probe, holder) = probe_holding_first_derivation();
    let (temp, state) = with_probe(Arc::clone(&probe));
    let first = refresh_request(&state);
    holder.entered.await.unwrap();

    let joined: Vec<_> = (0..8).map(|_| refresh_request(&state)).collect();
    // 八个请求全部被受理之后才放行，确保它们确实撞上在途派生而不是先后错开。
    probe.admissions_reach(9).await;
    holder.release.send(()).unwrap();

    completed(first).await;
    for request in joined {
        completed(request).await;
    }

    assert_eq!(
        probe.derivations(),
        2,
        "九个并发请求只换来在途一次加排队一次的真实派生"
    );
    assert_eq!(generation_of(&state).await, 3);
    let stats = state.refresh.stats();
    assert_eq!(stats.admitted, 9);
    assert_eq!(stats.runs_started, 2);
    assert_eq!(stats.coalesced(), 7);
    assert_eq!(stats.runs_failed, 0);
    assert_eq!(stats.settles_ignored, 0);
    assert!(!state.refresh.is_busy());
    drop(temp);
}

#[tokio::test]
async fn a_failed_derivation_is_reported_to_every_joined_request_and_the_next_one_starts_over() {
    let (probe, holder) = probe_holding_first_derivation();
    let (temp, state) = with_probe(Arc::clone(&probe));
    let first = refresh_request(&state);
    holder.entered.await.unwrap();

    let joined = refresh_request(&state);
    probe.admissions_reach(2).await;
    // 派生开始之后才让副本变得不可解析：两次派生都会读到它，两张票都该读到同一个失败。
    fs::write(vault_root(&temp).join("invalid.md"), [0xff, 0xfe]).unwrap();
    holder.release.send(()).unwrap();

    for request in [first, joined] {
        let (status, message) = failed(request).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        // 真实原因只进服务端日志；答复是固定说明，不回显库内路径。
        assert_eq!(message, crate::refresh::REFRESH_FAILED);
    }
    assert_eq!(generation_of(&state).await, 1, "失败不发布半成品代次");

    fs::remove_file(vault_root(&temp).join("invalid.md")).unwrap();
    completed(refresh_request(&state)).await;
    assert_eq!(
        generation_of(&state).await,
        2,
        "失败之后闸门重新受理并跑完整派生"
    );
    assert!(!state.refresh.is_busy());
    drop(temp);
}

#[tokio::test]
async fn a_refresh_whose_runner_dies_fails_its_own_ticket_and_lets_the_queued_one_take_over() {
    let (probe, holder) = probe_holding_first_derivation();
    probe.dies_after_release.store(true, Ordering::SeqCst);
    let (temp, state) = with_probe(Arc::clone(&probe));
    let first = refresh_request(&state);
    holder.entered.await.unwrap();

    let queued = refresh_request(&state);
    probe.admissions_reach(2).await;
    // 运行者死亡：派生句柄随阻塞任务一起消失，永远不会被结算。
    holder.release.send(()).unwrap();

    let (status, message) = failed(first).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(message, "刷新任务已死亡");
    // 排队中的派生由等待者接手并跑完，而不是陪着一起失败。
    completed(queued).await;

    assert_eq!(generation_of(&state).await, 2);
    assert_eq!(probe.derivations(), 2, "接手只跑一遍，遗弃本身不产生派生");
    let stats = state.refresh.stats();
    assert_eq!(stats.admitted, 2);
    assert_eq!(stats.runs_started, 2, "首次认领加一次接手");
    assert_eq!(stats.runs_abandoned, 1);
    assert_eq!(stats.settles_ignored, 0, "死亡的运行者没有迟到结算");
    assert!(!state.refresh.is_busy(), "在途位已随判定一并释放");

    completed(refresh_request(&state)).await;
    assert_eq!(
        generation_of(&state).await,
        3,
        "闸门没有被死亡的运行者永久卡住"
    );
    drop(temp);
}
