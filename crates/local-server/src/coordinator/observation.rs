//! 错误观测统一使旧代失效；不丢弃仍可供人查看的完整视图。

use super::Core;
use crate::LocalError;
use std::sync::Arc;

impl Core {
    pub(super) fn observe_error(self: &Arc<Self>, error: &LocalError) {
        use studiolo_vault_broker::BrokerError;
        let reason = match error {
            LocalError::Configuration => "binding_changed",
            LocalError::Read(BrokerError::MutationUnresolved) => "unresolved",
            LocalError::Read(
                BrokerError::RootChanged
                | BrokerError::WriterBindingMismatch
                | BrokerError::CorruptIdentity,
            ) => "binding_changed",
            LocalError::Read(
                BrokerError::CorruptWriterState | BrokerError::WriterLimitExceeded,
            ) => "writer_state_invalid",
            // 私有写者目录或读取 I/O 故障都使新鲜度未知；保留原错误并安排重扫。
            LocalError::Read(BrokerError::Io { .. }) => "capture_failed",
            LocalError::Read(BrokerError::CaptureChanged | BrokerError::UnstableRead(_)) => {
                "changed"
            }
            LocalError::Read(BrokerError::NotFound(path))
                if self
                    .view
                    .snapshot()
                    .ok()
                    .is_some_and(|view| view.generation.model.document(path).is_some()) =>
            {
                "changed"
            }
            _ => return,
        };
        let _ = self.request(reason);
    }
}
