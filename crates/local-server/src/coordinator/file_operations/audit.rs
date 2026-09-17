//! 原 typed outbox 独立于正文恢复标志；所有真实 ACK 都校验原事件与根身份。
use crate::coordinator::Core;
use std::sync::atomic::Ordering;
use studiolo_vault_broker::{FileOperationStatus, FsVaultBroker};

impl Core {
    pub(in crate::coordinator) fn flush_file_operation_audit(
        &self,
        broker: &mut FsVaultBroker,
    ) -> Result<Option<FileOperationStatus>, ()> {
        let result = (|| {
            let ledger = self.content_audit.as_ref().ok_or(())?;
            for _ in 0..2 {
                let candidates = broker.pending_file_operation_audit(2).map_err(|_| ())?;
                if candidates.is_empty() {
                    break;
                }
                for candidate in candidates {
                    self.roots.revalidate().map_err(|_| ())?;
                    let (offset, hash) =
                        crate::file_operation_audit::append(ledger.as_ref(), candidate.event())
                            .map_err(|_| ())?;
                    self.roots.revalidate().map_err(|_| ())?;
                    broker
                        .validate_root_identities(
                            self.roots.vault_identity(),
                            self.roots.writer_data_identity(),
                        )
                        .map_err(|_| ())?;
                    broker
                        .confirm_file_operation_audit(candidate, offset, &hash)
                        .map_err(|_| ())?;
                }
            }
            broker.file_operation_status().map_err(|_| ())
        })();
        self.file_operation_pending.store(
            result.as_ref().map_or(true, |status| status.is_some()),
            Ordering::Release,
        );
        result
    }
}
