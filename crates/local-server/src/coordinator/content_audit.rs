//! 单一owner补记持久事件；只有权威回执与事件完全匹配才确认outbox。
use super::Core;
use std::sync::atomic::Ordering;
use studiolo_vault_broker::{ContentAuditStatus, FsVaultBroker};

impl Core {
    pub(super) fn flush_content_audit(
        &self,
        broker: &mut FsVaultBroker,
    ) -> Result<ContentAuditStatus, ()> {
        let result = (|| {
            let ledger = self.content_audit.as_ref().ok_or(())?;
            let candidates = broker.pending_content_audit(64).map_err(|_| ())?;
            for candidate in candidates {
                self.roots.revalidate().map_err(|_| ())?;
                let (offset, hash) =
                    crate::editing::audit::append(ledger.as_ref(), candidate.event())?;
                broker
                    .confirm_content_audit(candidate, offset, &hash)
                    .map_err(|_| ())?;
            }
            let status = broker.content_audit_status().map_err(|_| ())?;
            self.content_audit_healthy
                .store(status.pending == 0, Ordering::Release);
            self.content_restore_pending
                .store(status.restore_unconfirmed, Ordering::Release);
            Ok(status)
        })();
        if result.is_err() {
            self.content_audit_healthy.store(false, Ordering::Release);
            // 审计失败仍从持久阶段判断是否可能已恢复；状态读取失败按未知保守阻断。
            let pending = broker
                .content_audit_status()
                .map_or(true, |status| status.restore_unconfirmed);
            self.content_restore_pending
                .store(pending, Ordering::Release);
        }
        result
    }
}
