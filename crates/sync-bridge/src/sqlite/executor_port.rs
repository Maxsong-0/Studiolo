//! 生产持久层端口：把执行器的 `PlanJournal` 逐方法映射到本模块的 `BridgeState`。
//!
//! 只做映射，不放宽任何冻结语义：重试沿用持久化的原幂等键，退休序号永久拒绝，
//! 未知终态绝不写成回执，真实审计 append 在 SQL 事务之外、精确回执后才 CAS 保存 ACK
//! —— 这三条都由 `BridgeState` 自己保证，本文件不额外开任何旁路。
//!
//! 本文件不持有文件系统、网络或进程能力；一切落盘都经由已登记的固定私有 owner。

use super::{
    ActionStatus, BridgeError, BridgeState, CommonObservation, EffectReceipt, ExecutionCheck,
    NoEffectReason, ReceiptBackend, ReceiptOutcome,
};
use crate::executor::{
    AuditProgress, CommonRecheck, ExecutionRecheck, IssuedJob, JobOutcome, JobRecord, JobState,
    JournalFault, JournalSnapshot, NoEffectCause, PlanJournal,
};
use crate::{Action, ActionKey, Change, Input, Side};
use studiolo_audit_ledger::AuditLedger;

/// 把执行器持久层端口接到真实 `BridgeState` 的生产适配器。
///
/// 三个协作者全部是显式构造参数，本类型没有任何隐式来源或环境权限：
/// `state` 是已打开并已核验绑定的持久 owner，`ledger` 是受信任的权威审计 adapter，
/// `input` 是本批冻结原输入 —— 它必须与持久层里冻结的那一份逐字段相同，
/// 否则 `BridgeState` 直接以 `RetryMismatch` 拒绝，本适配器不做任何补正。
pub struct StateJournal<'a> {
    /// 真实持久状态。
    state: &'a mut BridgeState,
    /// 受信任的权威审计 adapter。
    ledger: &'a dyn AuditLedger,
    /// 本批冻结输入。
    input: Input,
}

/// `BridgeError` 到 `JournalFault` 的完整穷尽映射。
///
/// 这里刻意不写 catch-all：`BridgeError` 新增变体必须在这里显式表态，
/// 否则编译失败。执行器端没有对应类别的失败一律落在 `JournalFault::Unavailable`
/// —— 它的语义就是「拿不到权威结论」，绝不会被上层折叠成成功。
pub(crate) fn journal_fault(error: BridgeError) -> JournalFault {
    match error {
        // 一一对应的已知类别。
        BridgeError::VersionConflict => JournalFault::VersionConflict,
        BridgeError::NotFound => JournalFault::NotFound,
        BridgeError::Retired => JournalFault::Retired,
        BridgeError::Fused => JournalFault::Fused,
        BridgeError::IntentRequired => JournalFault::IntentRequired,
        BridgeError::OutcomeMismatch => JournalFault::OutcomeMismatch,
        BridgeError::StaleObservation => JournalFault::StaleObservation,
        BridgeError::Budget => JournalFault::Budget,
        BridgeError::BindingMismatch => JournalFault::BindingMismatch,
        BridgeError::NotSettled => JournalFault::NotSettled,
        BridgeError::AuditUnavailable => JournalFault::AuditUnavailable,
        BridgeError::RetentionRequired => JournalFault::RetentionRequired,
        // 存储层面的打开/身份/平台失败：执行器无从判断是否已提交，按未知处理。
        BridgeError::MissingStore
        | BridgeError::AlreadyExists
        | BridgeError::UnsafeStorage
        | BridgeError::Busy
        | BridgeError::UnsupportedPlatform
        | BridgeError::UnsupportedSchema
        | BridgeError::CorruptStore
        | BridgeError::Unavailable => JournalFault::Unavailable,
        // 执行器端没有对应类别的持久层状态；不擅自压进某个已知类别。
        BridgeError::Overflow
        | BridgeError::PendingUnresolved
        | BridgeError::RetryMismatch
        | BridgeError::OutcomeUnknown => JournalFault::Unavailable,
    }
}

fn job_state(status: &ActionStatus) -> JobState {
    match status {
        ActionStatus::IntentPending => JobState::IntentPending,
        ActionStatus::Reserved => JobState::Reserved,
        ActionStatus::OutcomeUnknown => JobState::Issued,
        ActionStatus::Known(receipt) => match receipt.outcome {
            ReceiptOutcome::Committed { .. } => JobState::Committed,
            ReceiptOutcome::NoEffect { .. } => JobState::NoEffect,
        },
    }
}

impl<'a> StateJournal<'a> {
    /// 按显式协作者装配一次执行所需的持久层端口。
    pub fn new(state: &'a mut BridgeState, ledger: &'a dyn AuditLedger, input: Input) -> Self {
        Self {
            state,
            ledger,
            input,
        }
    }
    /// 本批冻结原输入；执行中途不得替换，只读暴露给装配方核对。
    pub fn frozen_input(&self) -> &Input {
        &self.input
    }
    fn action(&self, key: &ActionKey) -> Result<Action, JournalFault> {
        self.state
            .retry_pending(&self.input)
            .map_err(journal_fault)?
            .into_iter()
            .map(|view| view.action)
            .find(|action| &action.key == key)
            .ok_or(JournalFault::NotFound)
    }
}

impl PlanJournal for StateJournal<'_> {
    fn snapshot(&self) -> Result<JournalSnapshot, JournalFault> {
        let view = self.state.view().map_err(journal_fault)?;
        let jobs = match self.state.retry_pending(&self.input) {
            Ok(views) => views
                .into_iter()
                .map(|view| JobRecord {
                    key: view.action.key,
                    external_key: view.external_key,
                    state: job_state(&view.status),
                })
                .collect(),
            // 尚无保留计划或本序号已退休：逐动作面为空，整批结论交给上面的 view。
            Err(BridgeError::NotFound | BridgeError::Retired) => Vec::new(),
            Err(error) => return Err(journal_fault(error)),
        };
        Ok(JournalSnapshot {
            version: view.version,
            pending_sequence: view.pending_sequence,
            baseline_version: view.baseline.version,
            retired_through: view.retired_through,
            fused: view.fused,
            jobs,
        })
    }
    fn deliver_audit(&mut self, max_records: usize) -> Result<AuditProgress, JournalFault> {
        let delivery = self
            .state
            .flush_audit(self.ledger, max_records)
            .map_err(journal_fault)?;
        Ok(AuditProgress {
            acknowledged: delivery.acknowledged,
            remaining: delivery.remaining,
        })
    }
    fn issue(
        &mut self,
        key: &ActionKey,
        recheck: &ExecutionRecheck,
    ) -> Result<IssuedJob, JournalFault> {
        let check = ExecutionCheck {
            binding: recheck.binding.clone(),
            source: recheck.source.clone(),
            target: recheck.target.clone(),
            canonical_observation: recheck.canonical_observation,
            mirror_observation: recheck.mirror_observation,
            process_epoch: recheck.witness_epoch,
            expires_at: recheck.expires_at,
        };
        let issued = self.state.issue(key, check).map_err(journal_fault)?;
        Ok(IssuedJob {
            key: issued.action.key,
            external_key: issued.external_key,
            request_digest: issued.request_digest,
        })
    }
    fn record_outcome(
        &mut self,
        job: &IssuedJob,
        outcome: &JobOutcome,
    ) -> Result<(), JournalFault> {
        let action = self.action(&job.key)?;
        let path = action.key.path.as_str().to_owned();
        // 前置 revision 是冻结条件，只能来自原计划，绝不采信后端自报。
        let previous = match &action.change {
            Change::CreateOnly { .. } => None,
            Change::Replace {
                target_expected, ..
            }
            | Change::Delete { target_expected } => Some(target_expected.revision),
        };
        let backend = match action.key.target {
            Side::Canonical => ReceiptBackend::Broker {
                canonical_root: self.input.scope.binding.canonical_root_token,
            },
            Side::Mirror => ReceiptBackend::Mirror {
                mirror_root: self.input.scope.binding.mirror_root_token,
            },
        };
        let outcome = match outcome {
            // 未知不是回执：这里拒绝，让原键保持 Issued 等下次按原键核对。
            JobOutcome::Unknown => return Err(JournalFault::OutcomeMismatch),
            JobOutcome::Committed {
                new_revision,
                content_hash,
                bytes,
                receipt_id,
            } => ReceiptOutcome::Committed {
                path,
                previous_revision: previous,
                new_revision: *new_revision,
                content_hash: *content_hash,
                bytes: *bytes,
                receipt_id: receipt_id.clone(),
            },
            JobOutcome::NoEffect {
                reason,
                terminal_id,
            } => ReceiptOutcome::NoEffect {
                path,
                expected_revision: previous,
                reason: match reason {
                    NoEffectCause::CasRejected => NoEffectReason::CasRejected,
                    NoEffectCause::CancelledBeforeInstall => NoEffectReason::CancelledBeforeInstall,
                },
                terminal_id: terminal_id.clone(),
            },
        };
        self.state
            .record_outcome(
                &job.key,
                EffectReceipt {
                    external_key: job.external_key.clone(),
                    request_digest: job.request_digest,
                    backend,
                    outcome,
                },
            )
            .map_err(journal_fault)
    }
    fn prepare_retirement(&mut self, expected_version: u64) -> Result<(), JournalFault> {
        self.state
            .prepare_retirement(expected_version)
            .map_err(journal_fault)
    }
    fn settle(
        &mut self,
        expected_version: u64,
        sequence: u64,
        common: Option<CommonRecheck>,
    ) -> Result<(), JournalFault> {
        let common = common.map(|c| CommonObservation {
            canonical: c.canonical,
            mirror: c.mirror,
            approved: c.approved,
            process_epoch: c.witness_epoch,
            expires_at: c.expires_at,
        });
        self.state
            .settle(expected_version, sequence, common)
            .map_err(journal_fault)
    }
}
