//! 准入后的完整授权在槽锁外执行；只有本次取得 Proceed 才承担释放责任。

use crate::{
    CallId, GatewayError, JobCapability, MemoryGateway,
    call_table::{CallAdmission, CallTable},
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

impl<Request: PartialEq, Response: Clone> CallTable<Request, Response> {
    pub(super) fn admit_checked(
        &self,
        key: (AttemptId, CallId),
        request: &Request,
        authorize: impl FnOnce() -> Result<(), GatewayError>,
    ) -> Result<CallAdmission<Response>, GatewayError> {
        // admit 返回时槽锁已释放；guard 和拒绝审计不能延长该临界区。
        let admission = self.admit(key.clone(), request)?;
        if let Err(error) = authorize() {
            if matches!(admission, CallAdmission::Proceed) {
                // 包括 guard、拒绝事件 ID、拒绝审计失败在内，都归还自己的占位。
                self.abandon(&key)?;
            }
            return Err(error);
        }
        Ok(admission)
    }
}
