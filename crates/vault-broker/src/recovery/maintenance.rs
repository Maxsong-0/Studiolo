//! 维护专用有界 proposed 读取；不向 CLI 暴露正文或任意读取回调。
use super::{PendingMutation, observation};
use crate::{
    BrokerError, FsVaultBroker, MaintenanceCurrent, MaintenanceTarget, MaintenanceTargetKind,
    Revision, mutation::record::Operation, writer::directory,
};
impl PendingMutation {
    pub(crate) fn maintenance_operation(&self) -> Operation {
        self.operation.clone()
    }
    pub(crate) fn maintenance_target(
        &self,
        broker: &FsVaultBroker,
    ) -> Result<MaintenanceTarget, BrokerError> {
        observation::check(broker, self)?;
        let proposed = self.maintenance_proposed(broker)?.is_some();
        let mut state = self.state.clone();
        // schema 启用不改变原请求证据，也不让第一次 prepare 自己使审阅失效。
        state.format_version = state.format_version.max(3);
        state.maintenance_enabled = true;
        let encoded = serde_json::to_vec(&(
            "maintenance-review-v1",
            state,
            self.current.maintenance_digest()?,
            self.stage.maintenance_digest()?,
            proposed,
        ))
        .map_err(|_| BrokerError::CorruptWriterState)?;
        observation::check(broker, self)?;
        Ok(MaintenanceTarget {
            review_id: crate::sha256(&encoded),
            request_fingerprint: self.operation.fingerprint.clone(),
            path: self.path().clone(),
            kind: match self.phase() {
                crate::WriterPhase::Prepared => MaintenanceTargetKind::Prepared,
                _ => MaintenanceTargetKind::Installing,
            },
            sequence: Some(self.sequence()),
            previous_revision: self.previous_revision(),
            target_revision: Some(self.target_revision()),
            file_operation_id: None,
            secondary: None,
            file_operation: None,
            current: self
                .current_revision()
                .map_or(MaintenanceCurrent::Missing, |revision| {
                    MaintenanceCurrent::Present { revision }
                }),
            proposed_available: proposed,
        })
    }
    pub(crate) fn maintenance_proposed(
        &self,
        broker: &FsVaultBroker,
    ) -> Result<Option<super::proposed::Proposed>, BrokerError> {
        observation::check(broker, self)?;
        super::run::validate_optional_stage(broker, &self.operation)?;
        let proposed = if self.stage.identity.is_some() {
            Some(super::proposed::Proposed::stored(
                broker,
                directory(&broker.root, ".workbench/staging")?,
                crate::blob::store::logical(&self.operation.stage.name)?,
                self.path(),
                self.target_revision(),
            )?)
        } else if self.operation.phase == crate::mutation::record::Phase::Installing
            && self.operation.recording.is_some()
        {
            let history = directory(&broker.data, "history")?;
            if crate::blob::store::exists(&history, &self.target_revision().to_string())? {
                Some(super::proposed::Proposed::stored(
                    broker,
                    history,
                    crate::blob::store::logical(&self.target_revision().to_string())?,
                    self.path(),
                    self.target_revision(),
                )?)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(value) = &proposed
            && (value.identity().bytes != self.operation.stage.size
                || value.identity().sha256 != self.operation.stage.digest)
        {
            return Err(BrokerError::CorruptWriterState);
        }
        observation::check(broker, self)?;
        Ok(proposed)
    }
}
pub(crate) fn verify_evidence(
    broker: &FsVaultBroker,
    id: &str,
    operation: &Operation,
    current: Option<Revision>,
) -> Result<(), BrokerError> {
    super::evidence::verify(broker, id, operation, current)
}
