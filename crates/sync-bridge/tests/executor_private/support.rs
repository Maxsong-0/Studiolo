//! B4 执行器契约夹具：固定证据、真实临时 SQLite 与冻结计划构造。
//! 内存审计与假端口用例不代表生产身份验收。

use crate::sqlite::*;
use crate::*;
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::LazyLock,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::AuditLedger;
use studiolo_core::VaultPath;

/// 固定正文长度跨过 64 KiB 帧上限，保证流式落库至少两帧。
const BODY_BYTES: usize = 70_000;

static BODIES: LazyLock<Vec<(Vec<u8>, FileVersion)>> = LazyLock::new(|| {
    (0..8_u8)
        .map(|n| {
            let body: Vec<u8> = (0..BODY_BYTES).map(|i| (i as u8) ^ n).collect();
            let version = FileVersion {
                revision: [n; 32],
                sha256: Sha256::digest(&body).into(),
                bytes: body.len() as u64,
            };
            (body, version)
        })
        .collect()
});

/// 第 n 份固定正文；同一 n 永远是同一串字节与同一内容地址。
pub fn body(n: u8) -> &'static [u8] {
    BODIES
        .get(usize::from(n))
        .map(|(body, _)| body.as_slice())
        .unwrap_or_default()
}
/// 第 n 份固定正文的冻结版本证据。
pub fn version(n: u8) -> FileVersion {
    BODIES
        .get(usize::from(n))
        .map(|(_, version)| version.clone())
        .unwrap_or_else(|| FileVersion {
            revision: [n; 32],
            sha256: [n; 32],
            bytes: 0,
        })
}
/// 显式存在的第 n 份正文。
pub fn present(n: u8) -> Presence {
    Presence::Present(version(n))
}
/// 第 n 条固定路径。
pub fn path(n: usize) -> VaultPath {
    VaultPath::parse(format!("项目/记录-{n:04}.md")).unwrap()
}
/// 本轮固定绑定。
pub fn binding() -> Binding {
    Binding {
        link_id: [1; 16],
        canonical_vault: [2; 16],
        remote_vault: [3; 16],
        canonical_root_token: [4; 32],
        mirror_root_token: [5; 32],
        scope_hash: [6; 32],
        scope_epoch: 1,
    }
}
/// 本轮固定预算。
pub fn limits() -> Limits {
    Limits::conservative(1_000_000)
}
/// 单侧完整观察。
pub fn inventory(side: Side, sequence: u64, entries: BTreeMap<VaultPath, Presence>) -> Inventory {
    Inventory {
        binding: binding(),
        side,
        sequence,
        complete: true,
        entries,
    }
}

/// 不依赖存储的冻结输入；`baseline_live` 为真时基线逐项存活。
pub fn raw_input(count: usize, previous: u64, baseline_live: bool) -> Input {
    let paths: Vec<VaultPath> = (0..count).map(path).collect();
    let side: BTreeMap<VaultPath, Presence> =
        paths.iter().map(|p| (p.clone(), present(1))).collect();
    let entries = if previous == 0 {
        BTreeMap::new()
    } else {
        paths
            .iter()
            .map(|p| {
                let value = if baseline_live {
                    present(1)
                } else {
                    Presence::Missing
                };
                (
                    p.clone(),
                    Pair {
                        canonical: value.clone(),
                        mirror: value,
                    },
                )
            })
            .collect()
    };
    Input {
        scope: ApprovedScope {
            binding: binding(),
            paths: paths.iter().cloned().collect(),
        },
        baseline: Baseline {
            binding: binding(),
            version: previous,
            canonical_observation: previous,
            mirror_observation: previous,
            entries,
        },
        canonical: inventory(Side::Canonical, previous + 1, side.clone()),
        mirror: inventory(Side::Mirror, previous + 1, side),
        cursor: PlanningCursor {
            binding: binding(),
            baseline_version: previous,
            last_plan_sequence: previous,
            last_canonical_observation: previous,
            last_mirror_observation: previous,
        },
        expected_baseline_version: previous,
        plan_sequence: previous + 1,
        limits: limits(),
    }
}

/// 从当前持久视图派生的冻结输入；两侧默认同内容。
pub fn input(store: &BridgeState, count: usize) -> Input {
    let view = store.view().unwrap();
    let paths: Vec<VaultPath> = (0..count).map(path).collect();
    let side: BTreeMap<VaultPath, Presence> =
        paths.iter().map(|p| (p.clone(), present(1))).collect();
    Input {
        scope: ApprovedScope {
            binding: binding(),
            paths: paths.iter().cloned().collect(),
        },
        baseline: view.baseline,
        canonical: inventory(
            Side::Canonical,
            view.cursor.last_canonical_observation + 1,
            side.clone(),
        ),
        mirror: inventory(Side::Mirror, view.cursor.last_mirror_observation + 1, side),
        expected_baseline_version: view.cursor.baseline_version,
        plan_sequence: view.cursor.last_plan_sequence + 1,
        cursor: view.cursor,
        limits: limits(),
    }
}

/// 纯规划器给出的冻结计划；本用例不应触发规划期熔断。
pub fn ready(input: &Input) -> Plan {
    match plan(input.clone(), None).unwrap() {
        Decision::Ready(plan) => *plan,
        Decision::FuseBlocked(_) => panic!("此夹具不应触发规划期删除熔断"),
    }
}

/// 直接铸造一个计划，用于复核执行器自己的整批门槛，不经过规划器。
pub fn forge(
    input: Input,
    actions: Vec<Action>,
    conflicts: Vec<Conflict>,
    stats: DeleteStats,
) -> Plan {
    let key = PlanKey {
        link_id: input.scope.binding.link_id,
        scope_epoch: input.scope.binding.scope_epoch,
        sequence: input.plan_sequence,
    };
    Plan {
        original: input,
        key,
        actions,
        agreements: Vec::new(),
        conflicts,
        stats,
    }
}

/// 一条镜像侧条件删除动作。
pub fn mirror_delete(input: &mut Input, index: usize, sequence: u64) -> Action {
    let path = path(index);
    input
        .canonical
        .entries
        .insert(path.clone(), Presence::Missing);
    Action {
        key: ActionKey {
            plan: PlanKey {
                link_id: binding().link_id,
                scope_epoch: binding().scope_epoch,
                sequence,
            },
            path,
            target: Side::Mirror,
            operation: OperationKind::Delete,
        },
        change: Change::Delete {
            target_expected: version(1),
        },
    }
}

/// 预先建立一个含 `count` 条存活路径的共同基线。
pub fn seed(store: &mut BridgeState, ledger: &dyn AuditLedger, count: usize) -> Input {
    let seed = input(store, count);
    store
        .reserve(store.view().unwrap().version, seed.clone())
        .unwrap();
    while store.flush_audit(ledger, 64).unwrap().remaining {}
    let entries: BTreeMap<VaultPath, Presence> = seed
        .scope
        .paths
        .iter()
        .map(|p| (p.clone(), present(1)))
        .collect();
    let observation = CommonObservation {
        canonical: inventory(
            Side::Canonical,
            seed.canonical.sequence + 1,
            entries.clone(),
        ),
        mirror: inventory(Side::Mirror, seed.mirror.sequence + 1, entries),
        approved: seed.scope.clone(),
        process_epoch: store.observation_epoch(),
        expires_at: Instant::now() + Duration::from_secs(60),
    };
    store
        .settle(
            store.view().unwrap().version,
            seed.plan_sequence,
            Some(observation),
        )
        .unwrap();
    seed
}

/// 0700 私有临时根；只删除自己建立的目录。
pub struct Fixture {
    /// 本夹具的真实绝对根路径。
    pub root: PathBuf,
}
impl Fixture {
    /// 建立一个新的私有临时根。
    pub fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let root = std::env::temp_dir().join(format!("studiolo-t048-executor-{suffix}"));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self {
            root: fs::canonicalize(root).unwrap(),
        }
    }
    /// 初始化一个新的持久状态。
    pub fn init(&self) -> BridgeState {
        BridgeState::initialize(&self.root, binding(), limits()).unwrap()
    }
    /// 重开既有持久状态。
    pub fn reopen(&self) -> BridgeState {
        BridgeState::open_existing(&self.root, binding()).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
