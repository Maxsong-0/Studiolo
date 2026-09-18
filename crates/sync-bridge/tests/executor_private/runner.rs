//! 把假端口装成一次完整执行；真实持久层由调用方选 BridgeState 适配器或纯内存替身。

use super::adapter::LoggingJournal;
use super::ports::*;
use super::support::*;
use crate::executor::*;
use crate::*;
use std::time::{Duration, Instant};

/// 一套可复用的端口组合。
pub struct Rig {
    /// 静止能力。
    pub quiescence: FakeQuiescence,
    /// 可信观察 adapter。
    pub witness: FakeWitness,
    /// 正文来源。
    pub content: FakeContent,
    /// 镜像落库出口。
    pub sink: FakeSink,
    /// 已 Issued 动作执行者。
    pub backend: FakeBackend,
    /// MirrorCAS 账本。
    pub cas: MirrorCas,
    /// 顺序日志。
    pub log: Log,
}

impl Rig {
    /// 按冻结输入装配一套默认全绿的端口。
    pub fn new(input: &Input, epoch: Digest, common: Option<Presence>) -> Self {
        let log = log();
        Self {
            quiescence: FakeQuiescence::idle(input.mirror.sequence),
            witness: FakeWitness {
                input: input.clone(),
                epoch,
                deadline: Instant::now() + Duration::from_secs(60),
                common,
                fault: None,
            },
            content: FakeContent::new(log.clone()),
            sink: FakeSink::new(log.clone()),
            backend: FakeBackend::new(log.clone()),
            cas: MirrorCas::new(CasLimits::internal()),
            log,
        }
    }
    /// 执行一次；窗口足够长，过期由专门用例单独构造。
    pub fn run(
        &mut self,
        plan: &Plan,
        journal: &mut dyn PlanJournal,
    ) -> Result<ExecutionReport, ExecutorError> {
        let now = Instant::now();
        execute(
            plan,
            ExecutionPorts {
                quiescence: &self.quiescence,
                journal,
                witness: &mut self.witness,
                content: &mut self.content,
                sink: &mut self.sink,
                backend: &mut self.backend,
                cas: &mut self.cas,
            },
            now,
            now + Duration::from_secs(60),
        )
    }
    /// 顺序日志快照。
    pub fn lines(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
    /// 日志中第一次出现该前缀的位置。
    pub fn first(&self, prefix: &str) -> Option<usize> {
        self.lines().iter().position(|l| l.starts_with(prefix))
    }
}

/// 用真实 BridgeState 作持久层执行一次。
pub fn with_store(
    store: &mut crate::sqlite::BridgeState,
    ledger: &dyn studiolo_audit_ledger::AuditLedger,
    input: &Input,
    plan: &Plan,
    rig: &mut Rig,
) -> Result<ExecutionReport, ExecutorError> {
    let mut journal = LoggingJournal {
        inner: crate::sqlite::StateJournal::new(store, ledger, input.clone()),
        log: rig.log.clone(),
    };
    rig.run(plan, &mut journal)
}

/// 铸造一批镜像侧条件删除；`live` 决定原共同基线是否逐项存活。
pub fn forged_deletions(paths: usize, deletions: usize, live: bool) -> Plan {
    let mut input = raw_input(paths, 1, live);
    let sequence = input.plan_sequence;
    let mut actions = Vec::new();
    for index in 0..deletions {
        actions.push(mirror_delete(&mut input, index, sequence));
    }
    let stats = DeleteStats {
        canonical: 0,
        mirror: deletions as u64,
        baseline_live_paths: if live { paths as u64 } else { 0 },
        copy_bytes: 0,
    };
    forge(input, actions, Vec::new(), stats)
}
