//! 假端口与 BridgeState 适配器；顺序日志用于断言「保全在前、Issued 在中、回执在后」。

use super::support::*;
use crate::executor::*;
use crate::*;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
    time::{Duration, Instant},
};
use studiolo_core::VaultPath;

/// 跨端口共享的顺序日志。
pub type Log = Rc<RefCell<Vec<String>>>;

/// 新建空日志。
pub fn log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}
pub(super) fn note(log: &Log, line: String) {
    log.borrow_mut().push(line);
}

/// 静止能力假实现；绝不自行把未知改写成静止。
pub struct FakeQuiescence {
    /// 返回的判据。
    pub evidence: QuiescenceEvidence,
    /// 返回的镜像观察序号。
    pub observation: u64,
    /// 证据有效窗口终点。
    pub expires_at: Instant,
    /// 直接失败时返回的原因。
    pub fault: Option<QuiescenceFault>,
}
impl FakeQuiescence {
    /// 真正静止：受控子组空闲且 spawn 入口已关闭。
    pub fn idle(observation: u64) -> Self {
        Self {
            evidence: QuiescenceEvidence::ControlledSubgroupIdle {
                spawn_gate_closed: true,
            },
            observation,
            expires_at: Instant::now() + Duration::from_secs(60),
            fault: None,
        }
    }
    /// 取不到静止。
    pub fn refusing(fault: QuiescenceFault) -> Self {
        Self {
            fault: Some(fault),
            ..Self::idle(0)
        }
    }
}
impl MirrorQuiescence for FakeQuiescence {
    fn observe(
        &self,
        request: &QuiescenceRequest,
    ) -> Result<MirrorQuiescenceProof, QuiescenceFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        Ok(MirrorQuiescenceProof {
            link_id: request.link_id,
            scope_epoch: request.scope_epoch,
            mirror_root_token: request.mirror_root_token,
            evidence: self.evidence,
            mirror_observation: self.observation,
            expires_at: self.expires_at,
        })
    }
}

/// 可信观察 adapter 假实现；重验证据全部来自冻结输入。
pub struct FakeWitness {
    /// 冻结输入。
    pub input: Input,
    /// 当前打开实例标识。
    pub epoch: Digest,
    /// 证据截止点。
    pub deadline: Instant,
    /// 收尾共同观察的双侧内容；None 表示取不到完整共同观察。
    pub common: Option<Presence>,
    /// 直接失败时返回的原因。
    pub fault: Option<WitnessFault>,
}
impl ExecutionWitness for FakeWitness {
    fn recheck(
        &mut self,
        key: &ActionKey,
        _quiesced: &Quiesced,
    ) -> Result<ExecutionRecheck, WitnessFault> {
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        let (source, target) = match key.target {
            Side::Canonical => (
                self.input.mirror.entries.get(&key.path),
                self.input.canonical.entries.get(&key.path),
            ),
            Side::Mirror => (
                self.input.canonical.entries.get(&key.path),
                self.input.mirror.entries.get(&key.path),
            ),
        };
        Ok(ExecutionRecheck {
            binding: self.input.scope.binding.clone(),
            source: source.ok_or(WitnessFault::Incomplete)?.clone(),
            target: target.ok_or(WitnessFault::Incomplete)?.clone(),
            canonical_observation: self.input.canonical.sequence + 1,
            mirror_observation: self.input.mirror.sequence + 1,
            witness_epoch: self.epoch,
            expires_at: self.deadline,
        })
    }
    fn common(&mut self, _quiesced: &Quiesced) -> Result<Option<CommonRecheck>, WitnessFault> {
        let Some(value) = self.common.clone() else {
            return Ok(None);
        };
        let entries: BTreeMap<VaultPath, Presence> = self
            .input
            .scope
            .paths
            .iter()
            .map(|p| (p.clone(), value.clone()))
            .collect();
        Ok(Some(CommonRecheck {
            canonical: inventory(
                Side::Canonical,
                self.input.canonical.sequence + 1,
                entries.clone(),
            ),
            mirror: inventory(Side::Mirror, self.input.mirror.sequence + 1, entries),
            approved: self.input.scope.clone(),
            witness_epoch: self.epoch,
            expires_at: self.deadline,
        }))
    }
}

/// 正文来源模式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    /// 逐字节给出冻结版本。
    Exact,
    /// 字节数相同但内容被改过。
    Corrupt,
    /// 提前结束。
    Truncate,
    /// 来源不可用。
    Unavailable,
}

/// 正文来源假实现。
pub struct FakeContent {
    /// 内容地址到固定正文的登记表。
    pub registry: BTreeMap<Digest, &'static [u8]>,
    /// 当前模式。
    pub mode: ContentMode,
    /// 顺序日志。
    pub log: Log,
    current: Vec<u8>,
    cursor: usize,
}
impl FakeContent {
    /// 登记 0..8 号固定正文。
    pub fn new(log: Log) -> Self {
        let registry = (0..8_u8).map(|n| (version(n).sha256, body(n))).collect();
        Self {
            registry,
            mode: ContentMode::Exact,
            log,
            current: Vec::new(),
            cursor: 0,
        }
    }
}
impl MirrorContent for FakeContent {
    fn open(
        &mut self,
        side: Side,
        path: &VaultPath,
        version: &FileVersion,
    ) -> Result<(), TransferFault> {
        note(&self.log, format!("open {side:?} {}", path.as_str()));
        if self.mode == ContentMode::Unavailable {
            return Err(TransferFault::Unavailable);
        }
        let mut bytes = self
            .registry
            .get(&version.sha256)
            .ok_or(TransferFault::SourceMissing)?
            .to_vec();
        match self.mode {
            ContentMode::Corrupt => {
                if let Some(first) = bytes.first_mut() {
                    *first ^= 0xFF;
                }
            }
            ContentMode::Truncate => bytes.truncate(bytes.len() / 2),
            _ => {}
        }
        self.current = bytes;
        self.cursor = 0;
        Ok(())
    }
    fn next_frame(&mut self, buffer: &mut [u8]) -> Result<usize, TransferFault> {
        let rest = self
            .current
            .get(self.cursor..)
            .ok_or(TransferFault::Truncated)?;
        let take = rest.len().min(buffer.len());
        let (head, _) = buffer.split_at_mut(take);
        head.copy_from_slice(rest.get(..take).ok_or(TransferFault::Truncated)?);
        self.cursor += take;
        Ok(take)
    }
}

/// 受控镜像落库出口假实现。
pub struct FakeSink {
    /// 顺序日志。
    pub log: Log,
    /// accept 阶段是否直接拒绝。
    pub reject: bool,
    /// 已封口的内容地址。
    pub finished: Vec<ObjectId>,
}
impl FakeSink {
    /// 新建一个正常出口。
    pub fn new(log: Log) -> Self {
        Self {
            log,
            reject: false,
            finished: Vec::new(),
        }
    }
}
impl MirrorObjectSink for FakeSink {
    fn begin(&mut self) -> Result<(), TransferFault> {
        note(&self.log, "sink.begin".into());
        Ok(())
    }
    fn accept(&mut self, frame: &[u8]) -> Result<(), TransferFault> {
        if self.reject {
            return Err(TransferFault::Rejected);
        }
        note(&self.log, format!("sink.accept {}", frame.len()));
        Ok(())
    }
    fn finish(&mut self, id: ObjectId) -> Result<(), TransferFault> {
        note(&self.log, "sink.finish".into());
        self.finished.push(id);
        Ok(())
    }
    fn discard(&mut self) -> Result<(), TransferFault> {
        note(&self.log, "sink.discard".into());
        Ok(())
    }
}

/// 后端模式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BackendMode {
    /// 给出可核验的提交回执。
    Commit,
    /// 给出明确无效果回执。
    Reject,
    /// 拿不到权威终态。
    Unknown,
}

/// 已 Issued 动作执行者假实现。
pub struct FakeBackend {
    /// 当前模式。
    pub mode: BackendMode,
    /// 顺序日志。
    pub log: Log,
    /// 本次收到的暂存地址。
    pub staged: Vec<Option<ObjectId>>,
}
impl FakeBackend {
    /// 新建一个提交型后端。
    pub fn new(log: Log) -> Self {
        Self {
            mode: BackendMode::Commit,
            log,
            staged: Vec::new(),
        }
    }
}
impl JobBackend for FakeBackend {
    fn apply(&mut self, job: &IssuedJob, action: &Action, staged: Option<ObjectId>) -> JobOutcome {
        note(
            &self.log,
            format!("backend.apply staged={}", staged.is_some()),
        );
        self.staged.push(staged);
        match self.mode {
            BackendMode::Unknown => JobOutcome::Unknown,
            BackendMode::Reject => JobOutcome::NoEffect {
                reason: NoEffectCause::CasRejected,
                terminal_id: "fake-terminal-1".into(),
            },
            BackendMode::Commit => {
                let (new_revision, content_hash, bytes) = match &action.change {
                    Change::Delete { .. } => (None, None, None),
                    Change::CreateOnly { source } | Change::Replace { source, .. } => {
                        (Some([0xAA; 32]), Some(source.sha256), Some(source.bytes))
                    }
                };
                JobOutcome::Committed {
                    new_revision,
                    content_hash,
                    bytes,
                    receipt_id: format!("fake-receipt-{}", job.key.plan.sequence),
                }
            }
        }
    }
}
