//! owner 会话：把已批准能力、持久单槽与受控后端组合成一次一个运行的编排。
//!
//! 每次读取前后复验根 pins、配置摘要与代次；任何漂移都拒绝，不做部分继续。

use super::approval::{ApprovedTransport, PinnedRuntime, runtime_digest};
use super::audit::{
    AuditPayload, AuditPhaseKind, RequesterSource, RunAuditCandidate, binding_digest, event_id,
    outcome_summary,
};
use super::error::{Result, TransportError};
use super::machine::{self, PrepareRequest, Transition};
use super::state::{AuditReceipt, RunRecord, TransportState, decode, encode};
use super::tickets::{PreparedRun, TicketBinding};
use super::types::{ActiveSummary, RunId, RunOperation, RunStatus};
use crate::linux::ContainmentBackend;
use crate::store::StatePublisher;
use std::num::NonZeroU64;

/// 一次 owner 实例的完整会话；泛型只为把受控后端与纯逻辑分开测试。
///
/// 真实生产路径只有一个后端实现，且它需要 `VerifiedLinuxBackend` 才能构造。
pub(crate) struct RunSession<'scope, B: ContainmentBackend> {
    pub(crate) approval: &'scope dyn ApprovedTransport,
    pub(crate) runtime: PinnedRuntime,
    pub(crate) backend: B,
    pub(crate) publisher: Box<dyn StatePublisher>,
    pub(crate) epoch: [u8; 16],
    pub(crate) lease: [u8; 16],
    pub(crate) mirror_epoch: u64,
    pub(crate) released: Option<RunId>,
}

impl<'scope, B: ContainmentBackend> RunSession<'scope, B> {
    /// 打开既有槽：复验绑定、根、运行时与运行身份，并取一个新的实例 epoch。
    pub(crate) fn open(
        approval: &'scope dyn ApprovedTransport,
        runtime: PinnedRuntime,
        backend: B,
        publisher: Box<dyn StatePublisher>,
    ) -> Result<Self> {
        let mut epoch = [0u8; 16];
        let mut lease = [0u8; 16];
        random(&mut epoch)?;
        random(&mut lease)?;
        let session = Self {
            approval,
            runtime,
            backend,
            publisher,
            epoch,
            lease,
            mirror_epoch: 0,
            released: None,
        };
        let state = session.load()?;
        // owner 重开即换 epoch：旧实例发出的任何票据都立刻失效。
        Ok(Self {
            mirror_epoch: state.generation,
            ..session
        })
    }

    /// 读取并复验完整持久记录。
    pub(crate) fn load(&self) -> Result<TransportState> {
        self.approval.revalidate()?;
        self.publisher.check()?;
        let state = decode(&self.publisher.read()?)?;
        self.matches_approval(&state)?;
        self.publisher.check()?;
        self.approval.revalidate()?;
        Ok(state)
    }

    fn matches_approval(&self, state: &TransportState) -> Result<()> {
        if state.binding != self.approval.binding()? {
            return Err(TransportError::BindingMismatch);
        }
        if state.roots != self.approval.roots() {
            return Err(TransportError::RootsChanged);
        }
        if runtime_digest(&state.runtime) != self.runtime.digest() {
            return Err(TransportError::RuntimeChanged);
        }
        if state.backend != self.approval.backend() {
            return Err(TransportError::BindingMismatch);
        }
        let (uid, gid) = self.publisher.owner_identity();
        if state.owner.uid != uid || state.owner.gid != gid {
            return Err(TransportError::UnsafeStorage);
        }
        Ok(())
    }

    /// 完整原子发布一份新记录。
    pub(crate) fn commit(&mut self, next: &TransportState) -> Result<()> {
        let bytes = encode(next)?;
        self.publisher.replace(&bytes)
    }

    /// 安全逻辑元数据；不含 stdout、token 或原配置。
    pub(crate) fn inspect(&self) -> Result<RunStatus> {
        let state = self.load()?;
        Ok(RunStatus {
            generation: state.generation,
            next_sequence: state.next_sequence,
            audit_pending: state.active.as_ref().is_some_and(RunRecord::audit_pending),
            active: state.active.as_ref().map(|record| ActiveSummary {
                run_id: record.run_id,
                sequence: record.sequence,
                operation: record.operation,
                observation: record.observation(),
            }),
            last_terminal: state.last_terminal,
        })
    }

    /// 准备一次运行：state/Intent 已可靠发布，但没有任何启动命令。
    pub(crate) fn prepare_once(
        &mut self,
        expected_sequence: u64,
        operation: RunOperation,
        requested_at_millis: u64,
    ) -> Result<PreparedRun> {
        let state = self.load()?;
        if state.active.is_some() {
            return Err(TransportError::RunActive);
        }
        let mut run_id = [0u8; 16];
        random(&mut run_id)?;
        let next = machine::prepare(
            &state,
            &PrepareRequest {
                run_id: RunId::from_bytes(run_id),
                operation,
                requested_at_millis,
                expected_sequence,
                budget: self.approval.budget(),
            },
        )?;
        self.commit(&next)?;
        // 新一轮准备先作废旧的静止能力。
        self.mirror_epoch = next.generation;
        self.released = None;
        Ok(PreparedRun {
            binding: self.ticket(&next, RunId::from_bytes(run_id), expected_sequence)?,
        })
    }

    /// 仅尚未 LaunchIssued 时可用；固定「未执行」Outcome 并持久 Cancelled。
    pub(crate) fn cancel_prepared(&mut self, prepared: PreparedRun) -> Result<()> {
        let state = self.load()?;
        self.check_ticket(&prepared.binding, &state)?;
        let next = machine::apply(&state, Transition::Cancel)?;
        self.commit(&next)
    }

    /// 当前待补审计候选；没有待补事件时返回 `None`。
    pub(crate) fn pending_audit(&self) -> Result<Option<RunAuditCandidate>> {
        let state = self.load()?;
        let Some(record) = state.active.as_ref() else {
            return Ok(None);
        };
        let Some(phase) = machine::pending_phase(record) else {
            return Ok(None);
        };
        Ok(Some(self.candidate(&state, record, phase)?))
    }

    /// 登记 owner 已核验的真实 AuditRecord 回执。
    ///
    /// 本 crate 不能自己判断某个 offset/hash 是否来自权威账本；它只保证「确认的那条
    /// 事件和当前待补事件完全一致」，真实性由可信装配核对完整记录后负责。
    pub(crate) fn confirm_audit(
        &mut self,
        candidate: &RunAuditCandidate,
        offset: u64,
        hash: [u8; 32],
    ) -> Result<()> {
        let state = self.load()?;
        self.check_ticket(&candidate.binding, &state)?;
        let record = state.active.as_ref().ok_or(TransportError::NotFound)?;
        let phase = machine::pending_phase(record).ok_or(TransportError::AuditMismatch)?;
        let expected = self.candidate(&state, record, phase)?;
        if expected.phase != candidate.phase
            || expected.event_id != candidate.event_id
            || expected.payload != candidate.payload
        {
            return Err(TransportError::AuditMismatch);
        }
        let offset = NonZeroU64::new(offset).ok_or(TransportError::AuditRejected)?;
        if hash == [0u8; 32] {
            return Err(TransportError::AuditRejected);
        }
        if record
            .intent
            .receipt
            .is_some_and(|receipt| receipt.offset >= offset)
        {
            return Err(TransportError::AuditRejected);
        }
        let next = machine::apply(
            &state,
            Transition::Acknowledge {
                phase,
                receipt: AuditReceipt { offset, hash },
            },
        )?;
        self.commit(&next)
    }

    pub(crate) fn candidate(
        &self,
        state: &TransportState,
        record: &RunRecord,
        phase: AuditPhaseKind,
    ) -> Result<RunAuditCandidate> {
        Ok(RunAuditCandidate {
            binding: self.ticket(state, record.run_id, record.sequence)?,
            phase,
            event_id: event_id(record.run_id, phase),
            payload: AuditPayload {
                schema_version: 1,
                run_id: record.run_id,
                sequence: record.sequence,
                operation: record.operation,
                binding_digest: binding_digest(&state.binding),
                runtime_digest: self.runtime.digest(),
                backend_digest: state.backend.digest,
                budget: record.budget,
                requester: RequesterSource::OwnerSession,
                outcome: match phase {
                    AuditPhaseKind::Intent => None,
                    AuditPhaseKind::Outcome => Some(outcome_summary(record)),
                },
            },
        })
    }

    pub(crate) fn ticket(
        &self,
        state: &TransportState,
        run_id: RunId,
        sequence: u64,
    ) -> Result<TicketBinding> {
        Ok(TicketBinding {
            epoch: self.epoch,
            lease: self.lease,
            run_id,
            sequence,
            config_digest: state.binding.approved_config_digest,
            config_epoch: state.binding.scope_epoch,
            roots: state.roots.clone(),
            runtime_digest: self.runtime.digest(),
            issued_generation: state.generation,
            mirror_epoch: self.mirror_epoch,
        })
    }

    pub(crate) fn check_ticket(
        &self,
        ticket: &TicketBinding,
        state: &TransportState,
    ) -> Result<()> {
        ticket.check(state, self.epoch, self.lease, self.mirror_epoch)
    }
}

pub(crate) fn random(destination: &mut [u8]) -> Result<()> {
    getrandom::fill(destination).map_err(|_| TransportError::Unavailable)
}
