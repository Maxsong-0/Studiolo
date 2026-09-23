//! 只含元数据的 `Gateway` 封存保持 `raw` 隔离，也不占用最终产物清单。
use super::{authority::Authority, hash};
use std::{fs, path::Path, sync::Arc};
use studiolo_gateway::{
    CallId, RelayError, RelayGrant, RelayPolicyGuard, RelayProvider, RelayPublisher,
};
use studiolo_vault_broker::{GatewayCallSeal, GatewayPublisher};

pub(super) struct Publisher(pub Arc<Authority>);
impl RelayPublisher for Publisher {
    fn seal(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        raw: &Path,
        digest: &str,
    ) -> Result<(), RelayError> {
        let now = jiff::Timestamp::now().as_second();
        RelayPolicyGuard::inspect(self.0.as_ref(), grant, now)?;
        let metadata = fs::symlink_metadata(raw).map_err(|_| RelayError::Publication)?;
        if !metadata.is_file() || metadata.len() > u64::from(grant.limits.response_bytes) {
            return Err(RelayError::Publication);
        }
        let bytes = fs::read(raw).map_err(|_| RelayError::Publication)?;
        if hash(&bytes) != digest {
            return Err(RelayError::Publication);
        }
        let sources = serde_json::to_vec(&grant.sources).map_err(|_| RelayError::Publication)?;
        let seal = GatewayCallSeal {
            task_id: grant.capability.task_id(),
            attempt_id: grant.capability.attempt_id(),
            fence: grant.capability.fence(),
            call_id: call.as_str().into(),
            audit_intent: format!(
                "gateway.real.v1:{}:{}:{}",
                grant.capability.attempt_id(),
                grant.capability.fence().get(),
                hash(call.as_str().as_bytes())
            ),
            raw_sha256: digest.into(),
            raw_bytes: metadata.len(),
            source_manifest_sha256: hash(&sources),
            model_provider: match grant.provider {
                RelayProvider::Anthropic => "anthropic",
                RelayProvider::OpenAi => "openai",
            }
            .into(),
            model: grant.model.clone(),
            retain_until_unix_seconds: grant
                .sources
                .iter()
                .map(|s| s.retain_until_unix_seconds)
                .min()
                .ok_or(RelayError::Publication)?,
            snapshot_id: grant.snapshot_id.clone(),
            effective_policy_hash: grant.policy_hash.clone(),
        };
        let engine = self
            .0
            .board
            .engine
            .lock()
            .map_err(|_| RelayError::Publication)?;
        // 守卫检查与提交共用引擎锁，取消因此插不进二者之间。
        if !engine
            .control_db()
            .is_attempt_active_at(
                grant.capability.task_id(),
                grant.capability.attempt_id(),
                grant.capability.fence(),
                now.saturating_mul(1000),
            )
            .map_err(|_| RelayError::Publication)?
        {
            return Err(RelayError::Denied);
        }
        engine
            .broker()
            .access(|broker| broker.seal_gateway_call(seal))
            .map_err(|_| RelayError::Publication)?;
        Ok(())
    }
}
