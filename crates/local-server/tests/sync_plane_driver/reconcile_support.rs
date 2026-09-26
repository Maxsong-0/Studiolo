//! 原键核对契约的夹具：0700 私有临时根上的真实 `BridgeState`、内存审计、冻结输入与假核对端口。
//! 内存审计只验证顺序与 ACK 语义，不代表生产身份验收。

use super::fixture;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_core::VaultPath;
use studiolo_local_server::sync_plane::driver::reconcile::{
    MirrorTarget, ReconcilePort, ReconcileReport, reconcile_with,
};
use studiolo_local_server::sync_plane::job_backend::SeamVerdict;
use studiolo_sync_bridge::executor::{IssuedJob, JobOutcome, PlanJournal};
use studiolo_sync_bridge::sqlite::{
    ActionStatus, BridgeState, CommonObservation, ExecutionCheck, ReceiptOutcome, StateJournal,
    StateView,
};
use studiolo_sync_bridge::{
    Action, ApprovedScope, Change, FileVersion, Input, Inventory, Limits, Presence, Side,
};

/// 真实持久状态、内存审计与它们所在的私有临时根；字段按声明顺序析构，根最后删。
pub struct Store {
    pub state: BridgeState,
    pub ledger: MemoryAuditLedger,
    _root: tempfile::TempDir,
}

/// 新建一份空的真实持久状态。
pub fn store() -> Store {
    use std::os::unix::fs::DirBuilderExt;
    // 持久层只认 0700 私有根：在临时目录下显式按 0700 建一层，不依赖临时目录自身的权限。
    let root = tempfile::tempdir().expect("临时目录");
    let private = root.path().join("bridge");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&private)
        .expect("私有根");
    let path = std::fs::canonicalize(private).expect("规范路径");
    let state = BridgeState::initialize(&path, fixture::binding(), limits()).expect("初始化");
    Store {
        state,
        ledger: MemoryAuditLedger::new(),
        _root: root,
    }
}

fn limits() -> Limits {
    Limits::conservative(1_000_000)
}

/// 第 n 份固定版本；revision、摘要都取 n，字节数固定 10。
pub fn version(n: u8) -> FileVersion {
    FileVersion {
        revision: [n; 32],
        sha256: [n; 32],
        bytes: 10,
    }
}

/// 第 n 份固定版本的「存在」。
pub fn present(n: u8) -> Presence {
    Presence::Present(version(n))
}

fn inventory(side: Side, sequence: u64, entries: BTreeMap<VaultPath, Presence>) -> Inventory {
    Inventory {
        binding: fixture::binding(),
        side,
        sequence,
        complete: true,
        entries,
    }
}

/// 从当前持久视图派生的冻结输入；每项是 `(路径, canonical, 镜像)`。
pub fn input(store: &Store, items: &[(&str, Presence, Presence)]) -> Input {
    let current = view(store);
    let path = |raw: &str| VaultPath::parse(raw).expect("合法路径");
    let left = items.iter().map(|(p, c, _)| (path(p), c.clone())).collect();
    let right = items.iter().map(|(p, _, m)| (path(p), m.clone())).collect();
    let cursor = current.cursor.clone();
    Input {
        scope: ApprovedScope {
            binding: fixture::binding(),
            paths: items.iter().map(|(p, _, _)| path(p)).collect(),
        },
        baseline: current.baseline,
        canonical: inventory(Side::Canonical, cursor.last_canonical_observation + 1, left),
        mirror: inventory(Side::Mirror, cursor.last_mirror_observation + 1, right),
        expected_baseline_version: cursor.baseline_version,
        plan_sequence: cursor.last_plan_sequence + 1,
        cursor,
        limits: limits(),
    }
}

/// 当前持久视图。
pub fn view(store: &Store) -> StateView {
    store.state.view().expect("持久视图")
}

fn drain(store: &mut Store) {
    while store
        .state
        .flush_audit(&store.ledger, 64)
        .expect("审计")
        .remaining
    {}
}

/// 预留、推 Intent 审计、逐动作持久化 Issued——执行器发出后端调用前的最后一步；此后结果未知。
pub fn issue_all(store: &mut Store, original: &Input) -> Vec<(Action, IssuedJob)> {
    let current = view(store).version;
    store
        .state
        .reserve(current, original.clone())
        .expect("预留");
    drain(store);
    let views = store.state.retry_pending(original).expect("原键视图");
    let mut issued = Vec::new();
    for progress in views {
        let key = progress.action.key.clone();
        let (source, target) = match key.target {
            Side::Canonical => (&original.mirror, &original.canonical),
            Side::Mirror => (&original.canonical, &original.mirror),
        };
        let check = ExecutionCheck {
            binding: fixture::binding(),
            source: source.entries[&key.path].clone(),
            target: target.entries[&key.path].clone(),
            canonical_observation: original.canonical.sequence + 1,
            mirror_observation: original.mirror.sequence + 1,
            process_epoch: store.state.observation_epoch(),
            expires_at: Instant::now() + Duration::from_secs(30),
        };
        let pending = store.state.issue(&key, check).expect("Issued");
        let job = IssuedJob {
            key,
            external_key: pending.external_key,
            request_digest: pending.request_digest,
        };
        issued.push((progress.action, job));
    }
    issued
}

/// 执行器同一路径：经 `StateJournal` 把一条终态写成回执。
pub fn record(store: &mut Store, original: &Input, job: &IssuedJob, outcome: &JobOutcome) {
    StateJournal::new(&mut store.state, &store.ledger, original.clone())
        .record_outcome(job, outcome)
        .expect("写回执");
}

/// 推完审计后按当前版本闭合计划；`common` 为 None 时保留旧共同基线。
pub fn settle(store: &mut Store, sequence: u64, common: Option<CommonObservation>) {
    drain(store);
    let current = view(store).version;
    store.state.settle(current, sequence, common).expect("闭合");
}

/// 两侧都等于 `value` 的完整共同重查。
pub fn common(store: &Store, original: &Input, value: &Presence) -> CommonObservation {
    let cursor = view(store).cursor;
    let paths = original.scope.paths.iter();
    let both: BTreeMap<_, _> = paths.map(|p| (p.clone(), value.clone())).collect();
    CommonObservation {
        canonical: inventory(
            Side::Canonical,
            cursor.last_canonical_observation + 1,
            both.clone(),
        ),
        mirror: inventory(Side::Mirror, cursor.last_mirror_observation + 1, both),
        approved: original.scope.clone(),
        process_epoch: store.state.observation_epoch(),
        expires_at: Instant::now() + Duration::from_secs(60),
    }
}

/// 与冻结源逐字段一致的 Broker 提交回执（形态照接缝按原键重放返回的原回执）。
pub fn commit(action: &Action) -> JobOutcome {
    let (Change::CreateOnly { source } | Change::Replace { source, .. }) = &action.change else {
        panic!("夹具只提交非删除动作");
    };
    JobOutcome::Committed {
        new_revision: Some(source.revision),
        content_hash: Some(source.sha256),
        bytes: Some(source.bytes),
        receipt_id: "broker-receipt-1".into(),
    }
}

/// 原键当前持久状态。
pub fn status(store: &Store, action: &Action) -> ActionStatus {
    store.state.lookup(&action.key).expect("原键").status
}

/// 持久状态是否已是提交回执。
pub fn committed(status: &ActionStatus) -> bool {
    let known = |r: &ReceiptOutcome| matches!(r, ReceiptOutcome::Committed { .. });
    matches!(status, ActionStatus::Known(receipt) if known(&receipt.outcome))
}

/// 计数简写。
pub fn counts(resolved: usize, still_unknown: usize) -> ReconcileReport {
    ReconcileReport {
        resolved,
        still_unknown,
    }
}

/// 跑一次核对编排。
pub fn run(port: &mut FakePort, store: &mut Store) -> ReconcileReport {
    reconcile_with(port, &mut store.state, &store.ledger).expect("核对")
}

/// 假核对端口：canonical 侧仿接缝「先按原键查重放，有原回执直接返回，无回执才安装」。
pub struct FakePort {
    pub recorded: Option<JobOutcome>,
    pub attached: bool,
    pub mirror: MirrorTarget,
    pub quiescent: bool,
    pub replayed: Vec<IssuedJob>,
    pub probed: usize,
    pub installs: usize,
}

impl FakePort {
    /// 没有原回执、写者接得上、镜像目标「其它」、静止成立。
    pub fn new() -> Self {
        Self {
            recorded: None,
            attached: true,
            mirror: MirrorTarget::Other,
            quiescent: true,
            replayed: Vec::new(),
            probed: 0,
            installs: 0,
        }
    }
}

impl ReconcilePort for FakePort {
    fn replay_canonical(&mut self, job: &IssuedJob, _: &Action) -> SeamVerdict {
        self.replayed.push(job.clone());
        match &self.recorded {
            Some(outcome) => SeamVerdict::Outcome(outcome.clone()),
            None => {
                self.installs += 1;
                SeamVerdict::Unknown
            }
        }
    }

    fn writer_attached(&mut self) -> bool {
        self.attached
    }

    fn mirror_target(&mut self, _: &Action) -> MirrorTarget {
        self.probed += 1;
        self.mirror
    }

    fn still_quiescent(&mut self) -> bool {
        self.quiescent
    }
}
