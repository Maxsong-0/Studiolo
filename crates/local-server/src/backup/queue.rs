//! 待办以账本为准：从备份事件里算出下一个序号、没做完的原键、最近一次尝试的时间，
//! 以及健康页要看的最近成功、最近失败、挂着的作业和最新的灾难恢复点。
//!
//! 这里只有纯逻辑，按账本顺序一条条喂记录；读账本和写账本都在 `producer.rs`。
//! 没有单独的队列文件：进程重启后把账本里的备份事件重读一遍就得到同样的结论。
//! 仓库抽查的事件转给 [`Checks`]，和作业共用同一个读到的位置。
use super::checks::Checks;
use super::evidence::{FailureMark, JobFailure, JobMark, OpenJob, RecoveryMark};
use std::collections::BTreeMap;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord,
};
use studiolo_backup::worker::{
    audit::AdmissionFact,
    job::{BACKUP_AUDIT_ACTION, backup_event_key, backup_intent},
    recovery_point::BackupTargetKind,
    repository_check::REPOSITORY_CHECK_ACTION,
    state::JobKey,
};

/// 有原意图、还没有结果的一条作业。
struct Open {
    key: JobKey,
    /// 原意图落账的时刻。
    since_ms: i64,
    admission: Option<AuditRecord>,
    /// 准入里的内容截止时刻（Unix 毫秒）；还没准入为 `None`。
    content_cutoff_ms: Option<i64>,
    /// 准入里的人侧账本截止 offset；还没准入为 `None`。
    audit_offset: Option<u64>,
}

/// 账本里备份事件的累计视图。
#[derive(Default)]
pub(super) struct Queue {
    scanned: u64,
    highest: u64,
    open: BTreeMap<u64, Open>,
    last_attempt_ms: Option<i64>,
    first_attempt_ms: Option<i64>,
    last_success: Option<JobMark>,
    last_failure: Option<FailureMark>,
    /// 最近收口的那条作业没成功。
    latest_failed: bool,
    /// 成功收口、有准入的作业里内容截止最晚的那条。
    recovery_point: Option<RecoveryMark>,
    /// 成功收口、有准入的作业里盖到的最大人侧账本 offset（写者旧记录打包的水位，BUG-051）。
    sealed_offset: Option<u64>,
    /// 仓库抽查的累计视图。
    checks: Checks,
}

impl Queue {
    /// 已经读到账本的哪个位置；下一次从它后面接着读。
    pub(super) fn scanned(&self) -> u64 {
        self.scanned
    }

    /// 按账本顺序喂一条记录；不是备份事件的只推进位置。
    pub(super) fn absorb(&mut self, target: BackupTargetKind, record: &AuditRecord) {
        self.scanned = self.scanned.max(record.offset());
        let event = record.event();
        if event.action() == REPOSITORY_CHECK_ACTION {
            self.checks.absorb(target, record);
            return;
        }
        if event.action() != BACKUP_AUDIT_ACTION {
            return;
        }
        let at = record.recorded_at().as_millisecond();
        if let Some(key) = backup_event_key(event) {
            let sequence = key.sequence.get();
            // 别的目标类别的作业也占序号：事件键里没有目标，序号必须全账本唯一。
            self.highest = self.highest.max(sequence);
            match event.phase() {
                AuditPhase::Intent => {
                    if backup_intent(&key, target).is_ok_and(|expected| &expected == event) {
                        self.last_attempt_ms = Some(at);
                        self.first_attempt_ms.get_or_insert(at);
                        self.open.insert(
                            sequence,
                            Open {
                                key,
                                since_ms: at,
                                admission: None,
                                content_cutoff_ms: None,
                                audit_offset: None,
                            },
                        );
                    }
                }
                AuditPhase::Outcome => {
                    // 只认收口了这里的作业的结果：别的目标类别、对不上原键的结果不算成功也不算失败。
                    if self.open.get(&sequence).is_some_and(|open| open.key == key)
                        && let Some(open) = self.open.remove(&sequence)
                    {
                        self.close(sequence, at, &open, event);
                    }
                }
            }
            return;
        }
        if event.phase() != AuditPhase::Intent {
            return;
        }
        let Ok(fact) = AdmissionFact::from_receipt(record) else {
            return;
        };
        if fact.target() != target {
            return;
        }
        if let Some(open) = self.open.get_mut(&fact.key().sequence.get())
            && &open.key == fact.key()
        {
            open.admission = Some(record.clone());
            open.content_cutoff_ms = Some(fact.content_cutoff().millis());
            open.audit_offset = Some(fact.audit_offset());
        }
    }

    /// 仓库抽查的累计视图。
    pub(super) fn checks(&self) -> &Checks {
        &self.checks
    }

    /// 下一个可用的序号；单调递增，不复用。
    pub(super) fn next_sequence(&self) -> Option<u64> {
        self.highest.checked_add(1)
    }

    /// 已准入、还没有结果的原键里最早的一个的准入回执。
    pub(super) fn next_pending(&self) -> Option<&AuditRecord> {
        self.open.values().find_map(|open| open.admission.as_ref())
    }

    /// 有原意图、没有准入也没有结果的原键：封存没做成、失败结果又没写进去留下的。
    pub(super) fn orphans(&self) -> Vec<JobKey> {
        self.open
            .values()
            .filter(|open| open.admission.is_none())
            .map(|open| open.key.clone())
            .collect()
    }

    /// 还有没做完的作业。
    pub(super) fn outstanding(&self) -> bool {
        !self.open.is_empty()
    }

    /// 最近一次尝试（原意图）落账的时刻。
    pub(super) fn last_attempt_ms(&self) -> Option<i64> {
        self.last_attempt_ms
    }

    /// 账本里第一次尝试（原意图）落账的时刻。
    pub(super) fn first_attempt_ms(&self) -> Option<i64> {
        self.first_attempt_ms
    }

    /// 最近一次成功的作业。
    pub(super) fn last_success(&self) -> Option<JobMark> {
        self.last_success
    }

    /// 最近一次没成功的作业，不管之后有没有成功过。
    pub(super) fn last_failure(&self) -> Option<FailureMark> {
        self.last_failure
    }

    /// 最近收口的那条作业没成功。
    pub(super) fn latest_failed(&self) -> bool {
        self.latest_failed
    }

    /// 最新的灾难恢复点；失败、未验证、认不出的结果和别的目标类别都不推进它。
    pub(super) fn recovery_point(&self) -> Option<RecoveryMark> {
        self.recovery_point
    }

    /// 最近一次成功封存盖到的人侧账本 offset。失败、未验证的作业不算；生产只用 [`Self::seal_mark`]。
    #[cfg(test)]
    pub(super) fn sealed_offset(&self) -> Option<u64> {
        self.sealed_offset
    }

    /// 打包用的整个水位：账本 offset 加上内容截止时刻（T-249）。
    pub(super) fn seal_mark(&self) -> Option<SealMark> {
        self.sealed_offset.map(|offset| SealMark {
            offset,
            content_cutoff_ms: self.recovery_point.map(|mark| mark.content_cutoff_ms),
        })
    }

    /// 还没收口的作业里序号最小、也就是最早的那条；序号单调递增。
    pub(super) fn oldest_open(&self) -> Option<OpenJob> {
        self.open.iter().next().map(|(sequence, open)| OpenJob {
            sequence: *sequence,
            since_ms: open.since_ms,
            admitted: open.admission.is_some(),
        })
    }

    /// 一条作业收口：按结果记成最近成功或最近失败；成功且有准入的推进恢复点。
    fn close(&mut self, sequence: u64, at_ms: i64, open: &Open, event: &AuditEvent) {
        match failure_of(event) {
            None => {
                self.last_success = Some(JobMark { sequence, at_ms });
                self.latest_failed = false;
                self.sealed_offset = self.sealed_offset.max(open.audit_offset);
                // 没有准入就不知道内容截止到哪：照样算一次成功，但不当恢复点（年龄算不出来）。
                // 取截止最晚的：恢复点只往前走，不因后收口的旧作业倒退。
                if let Some(cutoff) = open.content_cutoff_ms
                    && self
                        .recovery_point
                        .is_none_or(|mark| cutoff > mark.content_cutoff_ms)
                {
                    self.recovery_point = Some(RecoveryMark {
                        sequence,
                        content_cutoff_ms: cutoff,
                    });
                }
            }
            Some(failure) => {
                self.last_failure = Some(FailureMark {
                    sequence,
                    at_ms,
                    failure,
                });
                self.latest_failed = true;
            }
        }
    }
}

/// 写者旧记录打包的水位（BUG-051、T-249）：最近一次成功封存盖到的人侧账本 offset，和成功封存里最晚的内容截止时刻。
/// 两样都只往前走；有审计主体的记录按两份回执的 offset 判，没有的（看板投影）按请求时间早于截止时刻判。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(crate) struct SealMark {
    /// 人侧账本 offset。
    pub(crate) offset: u64,
    /// 内容截止时刻（Unix 毫秒）；成功的作业都没有准入时没有。
    pub(crate) content_cutoff_ms: Option<i64>,
}
impl SealMark {
    /// 现在这一刻的打包判据。
    pub(crate) fn cutoff(self, now_ms: i64) -> studiolo_vault_broker::ArchiveCutoff {
        studiolo_vault_broker::ArchiveCutoff {
            sealed_offset: self.offset,
            sealed_content_ms: self.content_cutoff_ms,
            now_ms,
        }
    }
}

/// 从头读一遍账本，交回最近一次成功封存的水位；离线维护命令用，不占生产者的进度。
pub(crate) fn seal_mark(ledger: &dyn AuditLedger) -> Result<Option<SealMark>, AuditError> {
    const PAGE: usize = 8192;
    let mut queue = Queue::default();
    loop {
        let page = ledger.query(
            &AuditFilter::default()
                .after_offset(queue.scanned())
                .with_limit(PAGE),
        )?;
        for record in &page {
            queue.absorb(super::producer::TARGET, record);
        }
        if page.len() < PAGE {
            return Ok(queue.seal_mark());
        }
    }
}

/// 结果事件说的是不是成功；`None` 是成功。成功必须写明对象齐全、校验通过、最终清单成对引用了这次检查点，
/// 和 worker 认「可恢复点」的判据（`RemoteVerification::is_recoverable`）一样（复审 A2-F1）；
/// 少一条就和读不出来的结果一样不当成功（未验证永远不写成已通过）。
fn failure_of(event: &AuditEvent) -> Option<JobFailure> {
    let value = event.payload().as_value();
    let reason = || value.get("reason").cloned();
    let proven = |name: &str| value.get(name) == Some(&serde_json::Value::Bool(true));
    match value.get("result").and_then(serde_json::Value::as_str) {
        Some("succeeded")
            if proven("objects_complete")
                && proven("checksum_verified")
                && proven("paired_checkpoint_reference") =>
        {
            None
        }
        Some("failed") => Some(
            reason()
                .and_then(|reason| serde_json::from_value(reason).ok())
                .map_or(JobFailure::Unrecognized, JobFailure::Failed),
        ),
        Some("unverified") => Some(
            reason()
                .and_then(|reason| serde_json::from_value(reason).ok())
                .map_or(JobFailure::Unrecognized, JobFailure::Unverified),
        ),
        _ => Some(JobFailure::Unrecognized),
    }
}

#[cfg(test)]
#[path = "queue_tests.rs"]
mod tests;
