//! 机器黑板 API。授权留在经认证的组合根里。
mod candidates;
mod creation;
mod handoff;
mod stopped;
use super::*;
use crate::board_types::{bounded, reject, strings, valid_hash, valid_pointer};
use studiolo_control_db::BoardDefinition;

impl<B: VaultBroker + ArtifactStager, L: AuditLedger> PersistentTaskEngine<B, L> {
    /// 创建经审计的黑板任务，带必填约束与服务端固定的策略。
    pub fn create_board_task(
        &mut self,
        request_id: &str,
        spec: BoardTaskSpec,
        hash: &str,
        now: i64,
    ) -> Result<TaskId, TaskEngineError> {
        let request = (request_id, &spec, hash);
        self.audit_board("task.create", &request, AuditPhase::Intent, None)?;
        let result = self.create_board_task_inner(request_id, spec.clone(), hash, None, now);
        self.audit_board(
            "task.create",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// 冻结经认证的创建者；即使该项之后进入终态也保留。
    pub fn create_board_task_by_instance(
        &mut self,
        request_id: &str,
        spec: BoardTaskSpec,
        hash: &str,
        instance: &str,
        now: i64,
    ) -> Result<TaskId, TaskEngineError> {
        if instance.trim().is_empty() || instance.len() > 256 {
            return Err(reject("invalid creator instance"));
        }
        let request = (request_id, &spec, hash, instance);
        self.audit_board("task.create", &request, AuditPhase::Intent, None)?;
        let result =
            self.create_board_task_inner(request_id, spec.clone(), hash, Some(instance), now);
        self.audit_board(
            "task.create",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// 经审计地认领单个指定项；过期检查在认领事务内完成。
    pub fn claim_board_task(
        &mut self,
        task: TaskId,
        instance: &str,
        lease: i64,
        now: i64,
    ) -> Result<BoardClaim, TaskEngineError> {
        let request = (task, instance, lease, now);
        self.audit_board("task.claim", &request, AuditPhase::Intent, None)?;
        let result = self.claim_board_task_inner(task, instance, lease, now);
        self.audit_board(
            "task.claim",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// 当前 fence 下经审计的结构化交接。
    pub fn put_handoff(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        handoff: Handoff,
        now: i64,
    ) -> Result<(), TaskEngineError> {
        let request = (task, attempt, fence, &handoff, now);
        self.audit_board("handoff.created", &request, AuditPhase::Intent, None)?;
        let result = self.put_handoff_inner(task, attempt, fence, handoff.clone(), now);
        self.audit_board(
            "handoff.created",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }
    /// 经审计的终态回执；接受前核验全部内容指针。
    pub fn return_board_receipt(
        &mut self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        receipt: BoardReceipt,
        now: i64,
    ) -> Result<BoardView, TaskEngineError> {
        let request = (task, attempt, fence, &receipt, now);
        self.audit_board("handoff.returned", &request, AuditPhase::Intent, None)?;
        let result = self.return_board_receipt_inner(task, attempt, fence, receipt.clone(), now);
        self.audit_board(
            "handoff.returned",
            &request,
            AuditPhase::Outcome,
            Some(result.is_ok()),
        )?;
        result
    }

    /// 为每次受保护写入核对实例归属与当前未过期的认领。
    pub fn owns_board_claim(
        &self,
        task: TaskId,
        attempt: AttemptId,
        fence: Fence,
        instance: &str,
        now: i64,
    ) -> Result<bool, TaskEngineError> {
        Ok(self.control_db.board_task(task).map_err(map_db)?.is_some()
            && self
                .control_db
                .attempt(task, attempt)
                .map_err(map_db)?
                .is_some_and(|a| a.worker_id == instance)
            && self
                .control_db
                .is_attempt_active_at(task, attempt, fence, now)
                .map_err(map_db)?)
    }

    /// 读取单项：比对当前策略，并附在用或最近一次的租约元数据。
    pub fn board_detail(
        &self,
        task_id: TaskId,
        current_policy_hash: &str,
    ) -> Result<Option<BoardView>, TaskEngineError> {
        let Some(board) = self.control_db.board_task(task_id).map_err(map_db)? else {
            return Ok(None);
        };
        let task = self
            .task(task_id)?
            .ok_or(TaskEngineError::NotFound("task"))?;
        let active = task
            .active_attempt
            .or(task.last_attempt)
            .map(|id| self.control_db.attempt(task_id, id).map_err(map_db))
            .transpose()?
            .flatten();
        let progress = active
            .as_ref()
            .map(|a| {
                self.control_db
                    .attempt_progress(a.attempt_id)
                    .map_err(map_db)
            })
            .transpose()?
            .flatten();
        let candidate_stages = self
            .control_db
            .runner_candidate_stages(task_id)
            .map_err(map_db)?;
        let candidate_receipt = active
            .as_ref()
            .map(|a| {
                self.control_db
                    .artifact_receipt(task_id, a.attempt_id)
                    .map_err(map_db)
            })
            .transpose()?
            .flatten()
            .map(serde_json::from_value)
            .transpose()?;
        Ok(Some(BoardView {
            uses_old_policy: board.definition.policy_hash != current_policy_hash,
            holder: active.as_ref().map(|a| a.worker_id.clone()),
            progress,
            attempt: active,
            candidate_stages,
            candidate_receipt,
            task,
            board,
        }))
    }

    /// 只列出所请求项目中的工作项，保留权威元数据。
    pub fn board_list(
        &self,
        project: &str,
        current_policy_hash: &str,
    ) -> Result<Vec<BoardView>, TaskEngineError> {
        let mut views = self.board_list_all(current_policy_hash)?;
        views.retain(|view| view.task.spec.project == project);
        Ok(views)
    }

    /// 只读的跨项目清单，按任务 id 稳定排序；每条与 [`Self::board_detail`] 同一投影，
    /// 项目可见性仍由调用方（已鉴权的组合根）负责。
    pub fn board_list_all(
        &self,
        current_policy_hash: &str,
    ) -> Result<Vec<BoardView>, TaskEngineError> {
        self.control_db
            .board_tasks()
            .map_err(map_db)?
            .into_iter()
            .map(|board| {
                self.board_detail(board.task_id, current_policy_hash)?
                    .ok_or(TaskEngineError::NotFound("board task"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiolo_audit_ledger::MemoryAuditLedger;
    use studiolo_core::VaultUuid;
    use studiolo_vault_broker::MemoryVaultBroker;

    const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    type Engine = PersistentTaskEngine<MemoryVaultBroker, MemoryAuditLedger>;

    fn engine() -> Engine {
        let vault = VaultUuid::new();
        Engine::new(
            ControlDb::open_in_memory(vault).unwrap(),
            MemoryVaultBroker::with_uuid(vault),
            MemoryAuditLedger::new(),
        )
    }

    fn spec(project: &str, title: &str) -> BoardTaskSpec {
        BoardTaskSpec {
            task: TaskSpec {
                role: "implementer".into(),
                adapter: "consumer".into(),
                input_paths: vec![],
                allow_web: vec![],
                allow_db: vec![],
                retry_policy: RetryPolicy {
                    max_attempts: 1,
                    backoff_seconds: 0,
                    retry_on: vec![],
                },
                timeout: 300,
                project: project.into(),
                parent: None,
                title: Some(title.into()),
                instruction: None,
            },
            acceptance_criteria: vec!["清单完整".into()],
            files_owned: vec![],
            max_children: 8,
            artifact_target: None,
        }
    }

    #[test]
    fn empty_board_lists_nothing_across_projects() {
        assert!(engine().board_list_all(HASH).unwrap().is_empty());
    }

    #[test]
    fn cross_project_listing_matches_details_and_project_listing() {
        let mut engine = engine();
        let mut created = Vec::new();
        for (key, project) in [
            ("a1", "项目/A"),
            ("b1", "项目/B"),
            ("a2", "项目/A"),
            ("c1", "项目/C"),
        ] {
            let id = engine
                .create_board_task(key, spec(project, key), HASH, 1)
                .unwrap();
            created.push((id, project));
        }
        let claimed = created[1].0;
        engine
            .claim_board_task(claimed, "holder", 10_000, 2)
            .unwrap();

        let all = engine.board_list_all(HASH).unwrap();
        let ids: Vec<_> = all.iter().map(|view| view.task.task_id).collect();
        let mut expected: Vec<_> = created.iter().map(|(id, _)| *id).collect();
        expected.sort();
        assert_eq!(ids, expected, "覆盖全部项目，按任务 id 稳定排序");
        for view in &all {
            assert_eq!(
                Some(view),
                engine
                    .board_detail(view.task.task_id, HASH)
                    .unwrap()
                    .as_ref(),
                "与 board_detail 同一投影"
            );
            assert!(!view.uses_old_policy);
        }
        let held = all
            .iter()
            .find(|view| view.task.task_id == claimed)
            .unwrap();
        assert_eq!(held.holder.as_deref(), Some("holder"));
        assert_eq!(held.task.status, TaskStatus::Running);

        for project in ["项目/A", "项目/B", "项目/C", "项目/无"] {
            let filtered: Vec<_> = all
                .iter()
                .filter(|view| view.task.spec.project == project)
                .cloned()
                .collect();
            assert_eq!(engine.board_list(project, HASH).unwrap(), filtered);
        }
        assert!(
            engine
                .board_list_all(&"b".repeat(64))
                .unwrap()
                .iter()
                .all(|view| view.uses_old_policy),
            "是否旧策略按每次调用的当前哈希比较"
        );
    }
}
