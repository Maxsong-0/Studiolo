//! 人侧保存及恢复的唯一owner：身份、范围、精确基线、持久事件和CAS安装。
mod errors;
mod restore;
use crate::editing::{MAX_BYTES, Problem, Replace};
use crate::{
    CanonicalWorkbench,
    auth_http::ContentPrincipal,
    coordinator::{Control, WatchHealth},
};
pub(super) use errors::session_error;
use errors::{audit_unavailable, mutation_error};
use std::sync::MutexGuard;
use studiolo_vault_broker::{
    BrokerError, FsVaultBroker, Mutation, MutationActor, MutationKind, MutationMetadata,
    MutationReceipt, VaultReader,
};

impl CanonicalWorkbench {
    pub(crate) fn replace(
        &self,
        principal: ContentPrincipal,
        mut input: Replace,
    ) -> Result<MutationReceipt, Problem> {
        let mut owner = self.core.owner.lock().map_err(|_| Problem::unavailable())?;
        let broker = owner.as_mut().ok_or_else(Problem::unavailable)?;
        self.core
            .roots
            .revalidate()
            .map_err(|_| Problem::unavailable())?;
        broker
            .validate_root_identities(
                self.core.roots.vault_identity(),
                self.core.roots.writer_data_identity(),
            )
            .map_err(|_| Problem::unavailable())?;
        principal.verify().map_err(session_error)?;
        let scope = self
            .core
            .roots
            .human_content()
            .filter(|_| self.editing_enabled())
            .ok_or_else(Problem::forbidden)?;
        if !scope.allows(input.vault, &input.path) || broker.vault_uuid() != input.vault {
            return Err(Problem::forbidden());
        }
        let (issuer, subject) = principal.actor();
        let actor = MutationActor::Human {
            issuer: issuer.into(),
            subject: subject.into(),
        };
        let material =
            serde_json::to_vec(&("human.content.v1", issuer, subject, input.vault, &input.key))
                .map_err(|_| Problem::unavailable())?;
        let key = format!("human-{}", crate::editing::digest(&material));
        let _ = self.core.flush_content_audit(broker);
        if let Some(source) = input.source {
            if let Some(receipt) = broker
                .restore_replay(&input.path, input.expected, source, &key, &actor)
                .map_err(|error| mutation_error(error, &input))?
            {
                if broker
                    .content_audit_status()
                    .map_or(true, |s| s.restore_unconfirmed)
                {
                    return Err(Problem::unknown());
                }
                return Ok(receipt);
            }
            let historical = broker
                .read_history(&input.path, source, MAX_BYTES as u64)
                .map_err(crate::history::read_error)?;
            input.content =
                String::from_utf8(historical.content).map_err(|_| Problem::forbidden())?;
        }
        if input.content.contains('\0') || !errors::valid_document(&input.path, &input.content) {
            return Err(Problem::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_document",
                "文档 frontmatter 无效，请修正后保存。",
            ));
        }
        let kind = input
            .source
            .map_or(MutationKind::Replace, |source_revision| {
                MutationKind::Restore { source_revision }
            });
        let metadata = MutationMetadata::new(actor, kind).map_err(|_| Problem::unavailable())?;
        let mutation = Mutation::replace(
            input.path.clone(),
            input.content.as_bytes().to_vec(),
            input.expected,
            key,
        );
        if input.source.is_none() {
            if let Some(receipt) = broker
                .mutation_replay_recorded(&mutation, &metadata)
                .map_err(|error| mutation_error(error, &input))?
            {
                return Ok(receipt);
            }
            if broker
                .content_audit_status()
                .map_or(true, |s| s.restore_unconfirmed)
            {
                return Err(audit_unavailable());
            }
        }
        let epoch = self.content_epoch(broker)?;
        let current = match broker.read_file(&input.path, MAX_BYTES as u64) {
            Ok(current) => current,
            Err(BrokerError::NotFound(_)) => {
                return Err(self.changed(broker, &principal, &mutation, metadata, &input, epoch));
            }
            Err(error) => return Err(errors::read_error(error)),
        };
        if current.revision != input.expected {
            return Err(self.changed(broker, &principal, &mutation, metadata, &input, epoch));
        }
        let before = String::from_utf8(current.content).map_err(|_| Problem::forbidden())?;
        if !scope.preserves_generated(&input.path, &before, &input.content) {
            return Err(Problem::forbidden());
        }
        if input.source.is_some() {
            return self.restore(broker, &principal, mutation, metadata, &input, epoch);
        }
        let control = self.content_gate(&principal, epoch)?;
        self.core.mark_stale(&control, "changed");
        let result = broker.mutate_recorded(mutation, metadata);
        drop(control);
        let _ = self.core.request("changed");
        let _ = self.core.flush_content_audit(broker);
        // 安装后也可能发现不安全节点；持久未决阶段优先于表面的403分类。
        if result.is_err()
            && broker
                .write_status()
                .map_or(true, |status| status.unresolved)
        {
            return Err(Problem::unknown());
        }
        result.map_err(|error| mutation_error(error, &input))
    }

    pub(super) fn content_epoch(&self, broker: &FsVaultBroker) -> Result<u64, Problem> {
        let status = broker.write_status().map_err(|_| Problem::unavailable())?;
        let control = self
            .core
            .control
            .lock()
            .map_err(|_| Problem::unavailable())?;
        let view = self
            .core
            .view
            .0
            .read()
            .map_err(|_| Problem::unavailable())?;
        if status.unresolved
            || control.closing
            || control.health != WatchHealth::Healthy
            || view
                .freshness
                .as_ref()
                .is_none_or(|freshness| freshness.stale)
            || view.generation.as_ref().is_none_or(|generation| {
                generation.invalidation_epoch != Some(control.epoch)
                    || generation.indexed_through != Some(status.committed_through)
            })
        {
            return Err(Problem::stale());
        }
        Ok(control.epoch)
    }

    fn content_gate(
        &self,
        principal: &ContentPrincipal,
        epoch: u64,
    ) -> Result<MutexGuard<'_, Control>, Problem> {
        principal.verify().map_err(session_error)?;
        self.core
            .roots
            .revalidate()
            .map_err(|_| Problem::unavailable())?;
        let control = self
            .core
            .control
            .lock()
            .map_err(|_| Problem::unavailable())?;
        if control.epoch != epoch || control.closing || control.health != WatchHealth::Healthy {
            return Err(Problem::stale());
        }
        Ok(control)
    }

    fn changed(
        &self,
        broker: &mut FsVaultBroker,
        principal: &ContentPrincipal,
        mutation: &Mutation,
        metadata: MutationMetadata,
        input: &Replace,
        epoch: u64,
    ) -> Problem {
        let control = match self.content_gate(principal, epoch) {
            Ok(c) => c,
            Err(e) => return e,
        };
        // 当前已不符基线，只记录CAS拒绝；即使随后回到基线，也不转而安装。
        let result = broker.check_precondition_recorded(mutation, metadata);
        drop(control);
        let _ = self.core.request("changed");
        let _ = self.core.flush_content_audit(broker);
        result
            .err()
            .map_or_else(Problem::stale, |error| mutation_error(error, input))
    }
}
