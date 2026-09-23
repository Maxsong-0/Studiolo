//! 输入快照：核对 Broker 与 Control DB 同属一个 Vault、Run 确认时捕获输入、认领前核对输入未变。
use super::*;

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    pub(super) fn verify_vault_binding(&self) -> Result<(), TaskEngineError> {
        if self.broker.vault_uuid() != self.control_db.bound_vault_uuid().map_err(map_db)? {
            return Err(TaskEngineError::Integrity(
                "Broker and Control DB belong to different Vaults".to_owned(),
            ));
        }
        Ok(())
    }

    pub(super) fn capture_inputs(
        &self,
        task_id: TaskId,
        spec: &TaskSpec,
    ) -> Result<Snapshot, TaskEngineError> {
        let mut paths = spec.input_paths.clone();
        paths.push(task_page_path(task_id)?);
        paths.sort();
        paths.dedup();
        let mut inputs = Vec::with_capacity(paths.len());
        for path in paths {
            let object = self.broker.read(&path).map_err(TaskEngineError::Broker)?;
            inputs.push(SnapshotInput {
                path,
                expected_revision: object.revision,
            });
        }
        let snapshot = self
            .broker
            .snapshot(&inputs)
            .map_err(TaskEngineError::Broker)?;
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)? {
            return Err(TaskEngineError::Integrity(
                "Broker snapshot belongs to another Vault".to_owned(),
            ));
        }
        Ok(snapshot)
    }

    pub(super) fn inputs_unchanged(
        &self,
        task: &TaskRecord,
        snapshot: &Snapshot,
    ) -> Result<bool, TaskEngineError> {
        let page = task_page_path(task.task_id)?;
        let comparison = json!({
            "status": "queued", "active_attempt": null, "last_attempt": null, "input_snapshot": null,
        });
        for input in &snapshot.files {
            let current = match self.broker.read(&input.path) {
                Ok(value) => value,
                Err(BrokerError::NotFound(_)) => return Ok(false),
                Err(error) => return Err(TaskEngineError::Broker(error)),
            };
            if input.path == page {
                projection::validate_task_page(&current.content, task.task_id)
                    .map_err(TaskEngineError::Projection)?;
                let prior = projection::apply_projection(&input.content, &comparison)
                    .map_err(TaskEngineError::Projection)?;
                let current = projection::apply_projection(&current.content, &comparison)
                    .map_err(TaskEngineError::Projection)?;
                if current != prior {
                    return Ok(false);
                }
            } else if current.revision != input.revision {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

pub(super) fn task_page_path(task_id: TaskId) -> Result<VaultPath, TaskEngineError> {
    VaultPath::parse(format!("ops/tasks/{task_id}.md"))
        .map_err(|error| TaskEngineError::Integrity(error.to_string()))
}

pub(super) fn snapshot_record(snapshot: &Snapshot) -> SnapshotRecord {
    SnapshotRecord {
        snapshot_id: snapshot.id.as_str().to_owned(),
        vault_uuid: snapshot.vault_uuid,
        entries: snapshot
            .files
            .iter()
            .map(|file| SnapshotEntryRecord {
                path: file.path.to_string(),
                revision: file.revision.to_string(),
                effective_label_json: json!({"classification": "restricted"}),
            })
            .collect(),
    }
}
