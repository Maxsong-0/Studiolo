//! 准入后的完整授权在槽锁外执行；只有本次取得 Proceed 才承担释放责任。

use crate::{
    CallId, GatewayError, JobCapability, MemoryGateway,
    call_table::{CallAdmission, CallTable, RequestIdentity},
};
use studiolo_audit_ledger::AuditEventId;
use studiolo_core::{AttemptId, ToolId};

pub(super) struct Authorization<'a> {
    pub(super) capability: &'a JobCapability,
    pub(super) tool: &'a ToolId,
    pub(super) audit_id: &'a AuditEventId,
    pub(super) call_id: &'a CallId,
    pub(super) action: &'static str,
}

impl Authorization<'_> {
    pub(super) fn check(&self, gateway: &MemoryGateway) -> Result<(), GatewayError> {
        if let Err(error) = gateway.authorize(self.capability, self.tool) {
            let denial_id = gateway.denial_audit_event_id(self.audit_id, &error)?;
            return Err(gateway.audit_denial(
                denial_id,
                self.action,
                self.capability,
                self.tool.clone(),
                self.call_id,
                error,
            ));
        }
        Ok(())
    }
}

impl<Response: Clone> CallTable<Response> {
    pub(super) fn admit_checked(
        &self,
        key: (AttemptId, CallId),
        identity: RequestIdentity,
        authorize: impl FnOnce() -> Result<(), GatewayError>,
    ) -> Result<CallAdmission<'_, Response>, GatewayError> {
        // admit 返回时槽锁已释放；guard 和拒绝审计不能延长该临界区。
        let admission = self.admit(key, identity)?;
        // Proceed 已持有守卫；回调返回 Err 或 panic 都自动归还本次占位。
        authorize()?;
        Ok(admission)
    }
}
