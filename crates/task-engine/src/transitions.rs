//! `TaskEngine` 接缝在 `PersistentTaskEngine` 上的实现：命令转移、认领、续租、取消确认与对账。
use super::*;

impl<B, L> TaskEngine for PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    fn create_task(&mut self, spec: TaskSpec, now_ms: i64) -> Result<TaskId, TaskEngineError> {
        spec.validate()?;
        let task_id = TaskId::new();
        let definition = serde_json::to_value(spec)?;
        self.control_db
            .create_task(task_id, &definition, now_ms)
            .map_err(map_db)?;
        Ok(task_id)
    }

    fn command(
        &mut self,
        task_id: TaskId,
        command: TaskCommand,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError> {
        let current = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        match (command, current.status) {
            (TaskCommand::Run, TaskStatus::Draft) => {
                if self
                    .control_db
                    .board_task(task_id)
                    .map_err(map_db)?
                    .is_some()
                {
                    return Err(TaskEngineError::InvalidTaskSpec(
                        "board tasks queue through claim_board_task",
                    ));
                }
                // Run 只要求本任务自己的投影已交付；别的任务的坏页不能挡住这条命令。
                self.project_task(task_id)?;
                let snapshot = self.capture_inputs(task_id, &current.spec)?;
                self.control_db
                    .queue_with_snapshot(task_id, &snapshot_record(&snapshot), now_ms)
                    .map_err(map_db)?;
            }
            (TaskCommand::Run, TaskStatus::Queued)
            | (TaskCommand::Cancel, TaskStatus::Cancelled | TaskStatus::CancelRequested) => {}
            (TaskCommand::Cancel, TaskStatus::Draft | TaskStatus::Queued | TaskStatus::Running) => {
                self.control_db
                    .cancel_task(task_id, now_ms)
                    .map_err(map_db)?;
            }
            (
                TaskCommand::Cancel,
                TaskStatus::Committing
                | TaskStatus::NeedsReview
                | TaskStatus::Approved
                | TaskStatus::Succeeded
                | TaskStatus::Failed
                | TaskStatus::Abandoned,
            ) => {
                return Err(TaskEngineError::AlreadyTerminal);
            }
            (TaskCommand::Run, status) | (TaskCommand::Cancel, status) => {
                return Err(TaskEngineError::InvalidTransition {
                    from: format!("{status:?}"),
                    operation: match command {
                        TaskCommand::Run => "run",
                        TaskCommand::Cancel => "cancel",
                    },
                });
            }
        }
        self.task(task_id)?.ok_or(TaskEngineError::NotFound("task"))
    }

    fn claim(&mut self, request: ClaimRequest) -> Result<Option<ClaimedTask>, TaskEngineError> {
        if request.worker_id.trim().is_empty() || request.lease_until_ms <= request.now_ms {
            return Err(TaskEngineError::InvalidTransition {
                from: "invalid claim input".to_owned(),
                operation: "claim",
            });
        }
        let Some(task) = self.control_db.next_queued().map_err(map_db)? else {
            return Ok(None);
        };
        let spec = decode_spec(&task)?;

        // 所有 Vault I/O 都在认领事务之前完成；队列从不选取新的输入字节。
        let snapshot_id: SnapshotId =
            serde_json::from_value(json!(task.input_snapshot.as_ref().ok_or_else(|| {
                TaskEngineError::Integrity("queued task has no confirmed snapshot".to_owned())
            })?))?;
        let snapshot = self
            .broker
            .load_snapshot(&snapshot_id)
            .map_err(TaskEngineError::Broker)?;
        let snapshot_record = snapshot_record(&snapshot);
        if snapshot.vault_uuid != self.control_db.bound_vault_uuid().map_err(map_db)?
            || self
                .control_db
                .snapshot(snapshot.id.as_str())
                .map_err(map_db)?
                .as_ref()
                != Some(&snapshot_record)
        {
            return Err(TaskEngineError::Integrity(
                "confirmed snapshot does not match Control DB metadata or Vault identity"
                    .to_owned(),
            ));
        }
        if !self.inputs_unchanged(&task, &snapshot)? {
            self.control_db
                .require_reconfirmation(task.task_id, request.now_ms)
                .map_err(map_db)?;
            return Err(TaskEngineError::NeedsReconfirmation(task.task_id));
        }
        let attempt = self
            .control_db
            .claim(
                task.task_id,
                &request.worker_id,
                snapshot_record,
                request.now_ms,
                request.lease_until_ms,
            )
            .map_err(map_db)?;
        let run_spec = RunSpec {
            task_id: task.task_id,
            attempt_id: attempt.attempt_id,
            fence: attempt.fence,
            budget: Default::default(),
            execution: None,
            instruction: spec
                .instruction
                .clone()
                .or(spec.title.clone())
                .unwrap_or_default(),
            inputs: snapshot
                .files
                .into_iter()
                .map(|file| RunInput {
                    path: file.path,
                    revision: file.revision.to_string(),
                    content: file.content,
                })
                .collect(),
        };
        Ok(Some(ClaimedTask {
            snapshot_id: attempt.snapshot_id,
            run_spec,
        }))
    }

    fn attach_runner(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        runner_ref: RunnerRef,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        let task = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        if runner_ref.adapter != task.spec.adapter
            || runner_ref.opaque_id.trim().is_empty()
            || runner_ref.pid.is_some()
            || runner_ref.start_time_ticks.is_some()
        {
            return Err(TaskEngineError::InvalidRunnerRef);
        }
        let encoded = serde_json::to_value(runner_ref)?;
        self.control_db
            .attach_runner(task_id, attempt_id, fence, &encoded, now_ms)
            .map_err(map_db)
    }

    fn heartbeat(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        lease_until_ms: i64,
        now_ms: i64,
    ) -> Result<(), TaskEngineError> {
        if lease_until_ms <= now_ms {
            return Err(TaskEngineError::InvalidTransition {
                from: "expired lease".to_owned(),
                operation: "heartbeat",
            });
        }
        self.control_db
            .heartbeat(task_id, attempt_id, fence, lease_until_ms, now_ms)
            .map_err(map_db)
    }

    fn report(
        &mut self,
        event_id: &str,
        report: RunnerReport,
        now_ms: i64,
    ) -> Result<ReportResult, TaskEngineError> {
        // 实现体与 T1 预留、T2 前滚同在 terminal_commit 模块。
        self.report_inner(event_id, report, now_ms)
    }

    fn reconcile(&mut self, now_ms: i64) -> Result<ReconcileReport, TaskEngineError> {
        self.verify_vault_binding()?;
        if !self.control_db.board_tasks().map_err(map_db)?.is_empty() {
            return Err(TaskEngineError::InvalidTaskSpec(
                "board recovery requires explicit reconcile_startup or reconcile_periodic",
            ));
        }
        self.migrate_terminal_bodies()?;
        let abandoned_attempts = self.control_db.abandon_inflight(now_ms).map_err(map_db)?;
        // M0 只运行进程内 Stub；进程重启本身已隔离了它的工作。
        let cancelling = self.control_db.cancelling_attempts().map_err(map_db)?;
        for attempt in &cancelling {
            self.control_db
                .acknowledge_cancel(attempt.task_id, attempt.attempt_id, attempt.fence, now_ms)
                .map_err(map_db)?;
        }
        let audit_outcomes = self.deliver_pending_outcomes(None, now_ms)?;
        let committing = self.control_db.committing_attempts().map_err(map_db)?;
        let mut committed_attempts = 0;
        for attempt in committing {
            let reservation = attempt.terminal.clone().ok_or_else(|| {
                TaskEngineError::Integrity(format!(
                    "committing attempt {} has no reservation",
                    attempt.attempt_id
                ))
            })?;
            self.roll_forward(&prior_identity(&attempt), &reservation, now_ms)?;
            committed_attempts += 1;
        }
        Ok(ReconcileReport {
            abandoned_attempts,
            cancelled_attempts: cancelling.len(),
            committed_attempts,
            audit_outcomes,
        })
    }

    fn acknowledge_cancel(
        &mut self,
        task_id: TaskId,
        attempt_id: AttemptId,
        fence: Fence,
        now_ms: i64,
    ) -> Result<TaskView, TaskEngineError> {
        self.control_db
            .acknowledge_cancel(task_id, attempt_id, fence, now_ms)
            .map_err(map_db)?;
        self.task(task_id)?.ok_or(TaskEngineError::NotFound("task"))
    }

    fn project_pending(&mut self) -> Result<usize, TaskEngineError> {
        self.verify_vault_binding()?;
        let pending = self.control_db.pending_outbox().map_err(map_db)?;
        self.project_events(pending)
    }

    fn task(&self, task_id: TaskId) -> Result<Option<TaskView>, TaskEngineError> {
        self.control_db
            .task(task_id)
            .map_err(map_db)?
            .map(|record| task_view(&record))
            .transpose()
    }
}
