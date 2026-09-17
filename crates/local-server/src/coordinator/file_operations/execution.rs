//! 大对象准备在最终Auth前结束；借用原票据提交，未Issued的漂移仍可持票据取消。
use super::*;
use std::sync::atomic::Ordering;
use studiolo_vault_broker::{FileOperationOutcome, FileOperationPhase, FileOperationTicket};

impl CanonicalWorkbench {
    pub(super) fn execute_file_operation(
        &self,
        broker: &mut FsVaultBroker,
        principal: &ContentPrincipal,
        input: &Input,
        request: FileOperationRequest,
        metadata: FileOperationMetadata,
        epoch: u64,
    ) -> Result<FileOperationReceipt, Problem> {
        let admission = self.rename_admission(broker, principal, input)?;
        let control = self.file_gate(broker, principal, input, epoch)?;
        self.core.mark_stale(&control, "unresolved");
        self.core
            .file_operation_pending
            .store(true, Ordering::Release);
        drop(control);
        // prepare完成完整L0/私有stage保全；此处尚未进入任何可见安装。
        let ticket = match broker.prepare_file_operation(request, metadata) {
            Ok(ticket) => ticket,
            Err(error) => {
                let _ = self.core.request("changed");
                let pending = broker
                    .write_status()
                    .map_or(true, |status| status.unresolved);
                self.core
                    .file_operation_pending
                    .store(pending, Ordering::Release);
                return Err(if pending {
                    problem::unknown()
                } else {
                    problem::before_issue(error)
                });
            }
        };
        if let Some(admission) = &admission
            && let Err(error) = admission.ticket(ticket.preview())
        {
            return Err(self.cancel_file_ticket(broker, ticket, error));
        }
        let acknowledged = self
            .core
            .content_audit
            .as_ref()
            .ok_or(())
            .and_then(|ledger| {
                crate::file_operation_audit::append(ledger.as_ref(), ticket.intent())
                    .map_err(|_| ())
            })
            .and_then(|(offset, hash)| {
                self.core.roots.revalidate().map_err(|_| ())?;
                broker
                    .confirm_file_operation_intent(&ticket, offset, &hash)
                    .map_err(|_| ())
            });
        if acknowledged.is_err() {
            return Err(self.cancel_file_ticket(broker, ticket, problem::audit_unavailable()));
        }
        let control = match self.file_gate(broker, principal, input, epoch) {
            Ok(control) => control,
            Err(error) => return Err(self.cancel_file_ticket(broker, ticket, error)),
        };
        self.core.mark_stale(&control, "changed");
        let result = broker.commit_file_operation(&ticket);
        let consistent = match (&admission, &result) {
            (Some(admission), Ok(receipt)) => {
                admission.receipt(receipt, &ticket.preview().operation_id)
            }
            _ => true,
        };
        drop(control);
        if result.is_err()
            && broker.file_operation_status().is_ok_and(|status| {
                status.is_some_and(|status| {
                    matches!(
                        status.phase,
                        FileOperationPhase::Preparing | FileOperationPhase::Prepared
                    )
                })
            })
        {
            let error = result
                .err()
                .map_or_else(problem::unknown, problem::before_issue);
            return Err(self.cancel_file_ticket(broker, ticket, error));
        }
        drop(ticket);
        let _ = self.core.request("changed");
        if self
            .core
            .flush_file_operation_audit(broker)
            .map_or(true, |status| status.is_some())
        {
            return Err(problem::unknown());
        }
        if !consistent {
            return Err(problem::unknown());
        }
        result
            .map_err(|_| problem::unknown())
            .and_then(problem::receipt)
    }

    fn cancel_file_ticket(
        &self,
        broker: &mut FsVaultBroker,
        ticket: FileOperationTicket,
        known: Problem,
    ) -> Problem {
        // 尽量先闭合原固定Intent；不能先消费一个尚未取得ACK的票据再丢掉取消能力。
        if self.core.flush_file_operation_audit(broker).is_err() {
            let _ = self.core.request("changed");
            return problem::unknown();
        }
        let cancelled = broker.cancel_file_operation(ticket);
        let flushed = self.core.flush_file_operation_audit(broker);
        let _ = self.core.request("changed");
        if matches!(cancelled, Ok(ref receipt) if matches!(receipt.outcome, FileOperationOutcome::Cancelled | FileOperationOutcome::Rejected))
            && matches!(flushed, Ok(None))
        {
            known
        } else {
            problem::unknown()
        }
    }
}
