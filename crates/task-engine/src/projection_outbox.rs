//! 投影接线：按序号经 Broker CAS 投递 outbox 中的受保护字段投影，页面级拒绝只隔离所属任务。
use super::*;

/// 一条投影事件在单轮里最多尝试的写入次数（契约 §6「每条事件最多尝试三次」）。
const MAX_PROJECTION_ATTEMPTS: usize = 3;

impl<B, L> PersistentTaskEngine<B, L>
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    /// 只投递单个任务的待投递事件，按序号顺序。
    pub(super) fn project_task(&mut self, task_id: TaskId) -> Result<usize, TaskEngineError> {
        self.verify_vault_binding()?;
        let pending = self
            .control_db
            .pending_outbox()
            .map_err(map_db)?
            .into_iter()
            .filter(|event| event.task_id == task_id)
            .collect();
        self.project_events(pending)
    }

    /// 按序号顺序投递事件。页面级拒绝会在本轮剩余部分隔离该任务
    /// （其后续事件保持待投递，维持单任务内的顺序），但不阻塞其他
    /// 任务；存储与 Broker 故障仍会中止整轮。
    pub(super) fn project_events(
        &mut self,
        events: Vec<OutboxRecord>,
    ) -> Result<usize, TaskEngineError> {
        let mut delivered = 0;
        let mut failures: Vec<ProjectionFailure> = Vec::new();
        for event in events {
            if failures
                .iter()
                .any(|failure| failure.task_id == event.task_id)
            {
                continue;
            }
            match self.project_event(&event) {
                Ok(()) => delivered += 1,
                Err(error) if is_page_rejection(&error) => failures.push(ProjectionFailure {
                    task_id: event.task_id,
                    sequence: event.sequence,
                    reason: error.to_string(),
                }),
                Err(error) => return Err(error),
            }
        }
        if failures.is_empty() {
            Ok(delivered)
        } else {
            Err(TaskEngineError::ProjectionIncomplete {
                delivered,
                failures,
            })
        }
    }

    fn project_event(&mut self, event: &OutboxRecord) -> Result<(), TaskEngineError> {
        if self
            .control_db
            .board_task(event.task_id)
            .map_err(map_db)?
            .is_some()
        {
            return Err(TaskEngineError::Projection(
                "board task events require project_board batch publication".into(),
            ));
        }
        {
            let page = task_page_path(event.task_id)?;
            let vault_uuid = self.broker.vault_uuid();
            let mut succeeded = false;
            // 幂等键的判别位：只在「前提当下仍然成立、拒绝只可能是旧键终态的重放」时推进，
            // 详见 `projection::key`。
            let mut generation: u32 = 0;
            for _ in 0..MAX_PROJECTION_ATTEMPTS {
                let mutation = match self.broker.read(&page) {
                    Ok(current) => {
                        projection::validate_task_page(&current.content, event.task_id)
                            .map_err(TaskEngineError::Projection)?;
                        let content =
                            projection::apply_projection(&current.content, &event.payload_json)
                                .map_err(TaskEngineError::Projection)?;
                        if content == current.content {
                            succeeded = true;
                            break;
                        }
                        let key = projection::key::mutation_key(
                            vault_uuid,
                            event.task_id,
                            event.sequence,
                            &page,
                            Precondition::Match(current.revision),
                            &content,
                            generation,
                        );
                        Mutation::replace(page.clone(), content, current.revision, key)
                    }
                    Err(BrokerError::NotFound(_))
                        if projected_status(&event.payload_json) == Some(TaskStatus::Draft) =>
                    {
                        let task = self
                            .control_db
                            .task(event.task_id)
                            .map_err(map_db)?
                            .ok_or(TaskEngineError::NotFound("projection task"))?;
                        let created = jiff::Timestamp::from_millisecond(task.created_at_ms)
                            .map_err(|error| TaskEngineError::Projection(error.to_string()))?
                            .to_string();
                        let content = projection::render_task_page(
                            event.task_id,
                            &decode_spec(&task)?,
                            &created,
                        )
                        .map_err(TaskEngineError::Projection)?;
                        let content = projection::apply_projection(&content, &event.payload_json)
                            .map_err(TaskEngineError::Projection)?;
                        let key = projection::key::mutation_key(
                            vault_uuid,
                            event.task_id,
                            event.sequence,
                            &page,
                            Precondition::MustNotExist,
                            &content,
                            generation,
                        );
                        Mutation::create(page.clone(), content, key)
                    }
                    Err(error) => return Err(TaskEngineError::Broker(error)),
                };
                let precondition = mutation.precondition;
                let expected_content = mutation.content.clone();
                match self.broker.mutate(mutation) {
                    Ok(receipt) => {
                        // 幂等回执只证明历史上写入过，并不证明外部编辑者
                        // 此后一直没有改动那些已安装的字节。
                        let installed = self.broker.read(&page).map_err(TaskEngineError::Broker)?;
                        if installed.revision == receipt.revision
                            && installed.content == expected_content
                        {
                            succeeded = true;
                            break;
                        }
                    }
                    Err(BrokerError::RevisionConflict { .. }) => {
                        // 拒绝要么是对当前状态的判定，要么是 Broker 为这把键封存的拒绝终态的
                        // 重放。前提当下仍然成立 ⇒ 只可能是重放：推进判别位换一把没被封死的
                        // 键，否则这条事件会永远撞在同一个终态上（T-028 起 CAS 拒绝按键终态）。
                        // 前提确实已变时不推进：新前提本身就会派生出新键。
                        if self.premise_still_holds(&page, precondition)? {
                            generation = generation.saturating_add(1);
                        }
                        continue;
                    }
                    Err(BrokerError::IdempotencyConflict) => {
                        // 同一把键被另一个请求占用：明确拒绝，绝不静默继承它的终态。
                        // 只隔离本任务，别的任务的投影照常交付。
                        return Err(TaskEngineError::Projection(format!(
                            "task {} projection idempotency key is already bound to a different request",
                            event.task_id
                        )));
                    }
                    Err(error) => return Err(TaskEngineError::Broker(error)),
                }
            }
            if !succeeded {
                return Err(TaskEngineError::Projection(format!(
                    "task {} changed during {MAX_PROJECTION_ATTEMPTS} projection attempts",
                    event.task_id
                )));
            }
            self.control_db
                .mark_outbox_delivered(event.sequence, jiff::Timestamp::now().as_millisecond())
                .map_err(map_db)?;
        }
        Ok(())
    }

    /// 本次写入的前提（不存在 / 指定 revision）当下是否仍然成立。
    ///
    /// 只用来解释一次 CAS 拒绝：前提仍成立时，这次拒绝不可能是对当前状态的判定，
    /// 只能是 Broker 重放同一把幂等键上的旧拒绝终态。
    fn premise_still_holds(
        &self,
        page: &VaultPath,
        precondition: Precondition,
    ) -> Result<bool, TaskEngineError> {
        match (self.broker.read(page), precondition) {
            (Ok(current), Precondition::Match(expected)) => Ok(current.revision == expected),
            (Ok(_), Precondition::MustNotExist) => Ok(false),
            (Err(BrokerError::NotFound(_)), Precondition::MustNotExist) => Ok(true),
            (Err(BrokerError::NotFound(_)), Precondition::Match(_)) => Ok(false),
            (Err(error), _) => Err(TaskEngineError::Broker(error)),
        }
    }
}

/// 由单个任务自身页面内容引起的失败：不得阻塞其他任务。
fn is_page_rejection(error: &TaskEngineError) -> bool {
    matches!(
        error,
        TaskEngineError::Projection(_)
            | TaskEngineError::NotFound("projection task")
            | TaskEngineError::Json(_)
    )
}

/// 投影载荷里的 task 状态：载荷是 Control DB 写出的 JSON，按公开枚举解析而不比较状态名字符串。
fn projected_status(payload: &serde_json::Value) -> Option<TaskStatus> {
    serde_json::from_value(payload.get("status")?.clone()).ok()
}
