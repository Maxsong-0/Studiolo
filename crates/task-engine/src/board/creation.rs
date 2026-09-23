//! 必填工作项校验、受信任创建与显式认领准备。
use super::*;
impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 创建符合必填 schema 的工作项，并冻结受信任策略代码提供的策略哈希。
    pub(super) fn create_board_task_inner(
        &mut self,
        request_id: &str,
        spec: BoardTaskSpec,
        trusted_policy_hash: &str,
        instance: Option<&str>,
        now: i64,
    ) -> Result<TaskId, TaskEngineError> {
        bounded(&spec)?;
        spec.task.validate_board()?;
        if request_id.trim().is_empty()
            || request_id.len() > 256
            || !valid_hash(trusted_policy_hash)
            || !strings(&spec.acceptance_criteria, true)
            || spec.max_children > 8
            || spec.files_owned.len() > 256
        {
            return Err(reject("invalid required board constraints"));
        }
        let mut criteria = spec.acceptance_criteria.clone();
        criteria.sort();
        if criteria.windows(2).any(|p| p[0] == p[1]) {
            return Err(reject("acceptance criteria must be unique"));
        }
        if spec.files_owned.iter().any(VaultPath::is_protected) {
            return Err(reject("protected paths cannot be owned"));
        }
        if spec
            .artifact_target
            .as_ref()
            .is_some_and(|s| s.trim().is_empty() || s.len() > 256)
        {
            return Err(reject("invalid artifact target"));
        }
        let definition = BoardDefinition {
            acceptance_criteria: spec.acceptance_criteria,
            files_owned: spec.files_owned,
            max_children: spec.max_children,
            policy_hash: trusted_policy_hash.into(),
            parent: spec.task.parent,
            artifact_target: spec.artifact_target,
        };
        self.control_db
            .create_board_task_by_instance(
                request_id,
                TaskId::new(),
                &serde_json::to_value(spec.task)?,
                &definition,
                instance,
                now,
            )
            .map_err(map_db)
    }

    /// 认领指定的黑板项；draft 的输入在原子认领之前冻结。
    pub(super) fn claim_board_task_inner(
        &mut self,
        task_id: TaskId,
        instance: &str,
        lease_until: i64,
        now: i64,
    ) -> Result<BoardClaim, TaskEngineError> {
        if instance.trim().is_empty() || instance.len() > 256 || lease_until <= now {
            return Err(reject("invalid claim identity or lease"));
        }
        let board = self
            .control_db
            .board_task(task_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("board task"))?;
        let mut task = self
            .control_db
            .task(task_id)
            .map_err(map_db)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        if let Some(active) = task.active_attempt {
            let attempt = self
                .control_db
                .attempt(task_id, active)
                .map_err(map_db)?
                .ok_or(TaskEngineError::NotFound("attempt"))?;
            if attempt.worker_id == instance
                && self
                    .control_db
                    .is_attempt_active_at(task_id, active, attempt.fence, now)
                    .map_err(map_db)?
            {
                return Ok(BoardClaim {
                    task: self
                        .board_detail(task_id, &board.definition.policy_hash)?
                        .ok_or(TaskEngineError::NotFound("board task"))?,
                    attempt,
                });
            }
        }
        if task.status == TaskStatus::Draft {
            let spec = decode_spec(&task)?;
            let inputs = spec
                .input_paths
                .iter()
                .map(|path| {
                    self.broker
                        .read(path)
                        .map(|object| SnapshotInput {
                            path: path.clone(),
                            expected_revision: object.revision,
                        })
                        .map_err(TaskEngineError::Broker)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let snapshot = self
                .broker
                .snapshot(&inputs)
                .map_err(TaskEngineError::Broker)?;
            self.control_db
                .queue_with_snapshot(task_id, &snapshot_record(&snapshot), now)
                .map_err(map_db)?;
            task = self
                .control_db
                .task(task_id)
                .map_err(map_db)?
                .ok_or(TaskEngineError::NotFound("task"))?;
        }
        let snapshot_id: SnapshotId = serde_json::from_value(json!(
            task.input_snapshot
                .as_ref()
                .ok_or_else(|| reject("task has no frozen input"))?
        ))?;
        let snapshot = self
            .broker
            .load_snapshot(&snapshot_id)
            .map_err(TaskEngineError::Broker)?;
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)?
            || self
                .control_db
                .snapshot(snapshot.id.as_str())
                .map_err(map_db)?
                .as_ref()
                != Some(&snapshot_record(&snapshot))
        {
            return Err(TaskEngineError::Integrity(
                "board snapshot metadata differs".into(),
            ));
        }
        if !self.inputs_unchanged(&task, &snapshot)? {
            self.control_db
                .require_reconfirmation(task_id, now)
                .map_err(map_db)?;
            return Err(TaskEngineError::NeedsReconfirmation(task_id));
        }
        let attempt = self
            .control_db
            .claim(
                task_id,
                instance,
                snapshot_record(&snapshot),
                now,
                lease_until,
            )
            .map_err(map_db)?;
        Ok(BoardClaim {
            task: self
                .board_detail(task_id, &board.definition.policy_hash)?
                .ok_or(TaskEngineError::NotFound("board task"))?,
            attempt,
        })
    }
}
