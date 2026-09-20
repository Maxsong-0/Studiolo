//! Blackboard metadata and atomic hierarchy/path arbitration. Artifact bodies never enter here.
mod schema;
use super::*;
pub(super) use schema::migrate;
use studiolo_core::VaultPath;

/// Validated machine work-item constraints, supplied only by TaskEngine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardDefinition {
    #[doc = "Ordered, individually testable acceptance criteria."]
    pub acceptance_criteria: Vec<String>,
    #[doc = "Exclusive logical paths, including directory prefixes; an empty list is read-only."]
    pub files_owned: Vec<VaultPath>,
    #[doc = "Maximum direct children, capped at eight."]
    pub max_children: u8,
    #[doc = "Trusted effective policy version frozen at creation."]
    pub policy_hash: String,
    #[doc = "Optional parent work item."]
    pub parent: Option<TaskId>,
    #[doc = "Optional mutually exclusive target within the project."]
    pub artifact_target: Option<String>,
}

/// Authority metadata accompanying the separately stored task definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardRecord {
    #[doc = "Stable work-item identity."]
    pub task_id: TaskId,
    #[doc = "Frozen machine constraints."]
    pub definition: BoardDefinition,
    #[doc = "Derived tree depth, with roots at zero and a hard maximum of three."]
    pub depth: u8,
    #[doc = "Authenticated creator retained after the task completes."]
    pub created_by_instance: Option<String>,
    #[doc = "Structured outbound handoff metadata."]
    pub handoff: Option<Value>,
    #[doc = "Structured terminal receipt metadata."]
    pub receipt: Option<Value>,
}

impl ControlDb {
    /// Atomically creates a board task, freezes trusted policy, and checks the parent tree.
    pub fn create_board_task(
        &mut self,
        request_id: &str,
        task_id: TaskId,
        task: &Value,
        definition: &BoardDefinition,
        now_ms: i64,
    ) -> Result<TaskId, DbError> {
        self.create_board_task_by_instance(request_id, task_id, task, definition, None, now_ms)
    }

    /// Creates metadata with an authenticated originating instance in the same transaction.
    pub fn create_board_task_by_instance(
        &mut self,
        request_id: &str,
        task_id: TaskId,
        task: &Value,
        definition: &BoardDefinition,
        instance: Option<&str>,
        now_ms: i64,
    ) -> Result<TaskId, DbError> {
        let request = serde_json::to_string(&(task, definition, instance))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT request_json,task_id FROM board_requests WHERE request_id=?1",
                [request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((stored, id)) = prior {
            if stored != request {
                return Err(DbError::IdempotencyMismatch);
            }
            return parse_task_id(&id);
        }
        if definition.acceptance_criteria.is_empty()
            || definition.max_children > 8
            || definition.policy_hash.is_empty()
        {
            return Err(DbError::BoardRejected("invalid required board constraints"));
        }
        let depth = if let Some(parent) = definition.parent {
            let (depth, limit, raw, status): (u8,u8,String,String) = tx.query_row(
                "SELECT b.depth,b.max_children,b.definition_json,t.status FROM board_tasks b JOIN tasks t USING(task_id) WHERE b.task_id=?1",
                [parent.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
                .optional()?.ok_or(DbError::NotFound("parent board task"))?;
            // Conservative delegation: create children before the parent takes any claim.
            if !matches!(status.as_str(), "draft" | "queued") {
                return Err(DbError::BoardRejected(
                    "parent must be unclaimed before delegation",
                ));
            }
            if depth >= 3 {
                return Err(DbError::BoardRejected("depth exceeds 3"));
            }
            let children: i64 = tx.query_row(
                "SELECT COUNT(*) FROM board_tasks WHERE parent=?1",
                [parent.to_string()],
                |r| r.get(0),
            )?;
            if children >= i64::from(limit) {
                return Err(DbError::BoardRejected("fanout exceeds max_children"));
            }
            let parent_project: String = tx.query_row(
                "SELECT json_extract(definition_json,'$.project') FROM tasks WHERE task_id=?1",
                [parent.to_string()],
                |r| r.get(0),
            )?;
            if task.get("project").and_then(Value::as_str) != Some(parent_project.as_str()) {
                return Err(DbError::BoardRejected("child project differs from parent"));
            }
            let parent_def: BoardDefinition = serde_json::from_str(&raw)?;
            if definition
                .files_owned
                .iter()
                .any(|p| !parent_def.files_owned.iter().any(|root| contains(root, p)))
            {
                return Err(DbError::BoardRejected("child paths exceed parent scope"));
            }
            let mut stmt = tx.prepare("SELECT definition_json FROM board_tasks WHERE parent=?1")?;
            for raw in stmt.query_map([parent.to_string()], |r| r.get::<_, String>(0))? {
                let sibling: BoardDefinition = serde_json::from_str(&raw?)?;
                if overlaps(&definition.files_owned, &sibling.files_owned).is_some() {
                    return Err(DbError::BoardRejected("sibling paths overlap"));
                }
            }
            depth + 1
        } else {
            0
        };
        tx.execute("INSERT INTO tasks(task_id,definition_json,status,created_at_ms,updated_at_ms) VALUES(?1,?2,'draft',?3,?3)",
            params![task_id.to_string(),serde_json::to_string(task)?,now_ms])?;
        tx.execute("INSERT INTO board_tasks(task_id,parent,depth,max_children,definition_json,created_by_instance) VALUES(?1,?2,?3,?4,?5,?6)",
            params![task_id.to_string(),definition.parent.map(|id|id.to_string()),depth,definition.max_children,serde_json::to_string(definition)?,instance])?;
        tx.execute(
            "INSERT INTO board_requests(request_id,request_json,task_id) VALUES(?1,?2,?3)",
            params![request_id, request, task_id.to_string()],
        )?;
        runtime_audit::enqueue(
            &tx,
            &format!("board-create:{task_id}"),
            "task.policy_pinned",
            serde_json::json!({"task_id":task_id,"policy_hash":definition.policy_hash,"depth":depth}),
        )?;
        enqueue_projection(&tx, task_id, &format!("create:{task_id}"))?;
        tx.commit()?;
        Ok(task_id)
    }

    /// Reads one board record without artifact bytes.
    pub fn board_task(&self, task_id: TaskId) -> Result<Option<BoardRecord>, DbError> {
        type Row = (String, u8, Option<String>, Option<String>, Option<String>);
        let row: Option<Row> = self.connection.query_row(
            "SELECT definition_json,depth,handoff_json,receipt_json,created_by_instance FROM board_tasks WHERE task_id=?1",
            [task_id.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        row.map(
            |(definition, depth, handoff, receipt, created_by_instance)| {
                Ok(BoardRecord {
                    task_id,
                    depth,
                    created_by_instance,
                    definition: serde_json::from_str(&definition)?,
                    handoff: handoff.map(|v| serde_json::from_str(&v)).transpose()?,
                    receipt: receipt.map(|v| serde_json::from_str(&v)).transpose()?,
                })
            },
        )
        .transpose()
    }

    /// Lists board tasks deterministically; callers apply authenticated project visibility.
    pub fn board_tasks(&self) -> Result<Vec<BoardRecord>, DbError> {
        let mut stmt = self
            .connection
            .prepare("SELECT task_id FROM board_tasks ORDER BY task_id")?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| {
                self.board_task(parse_task_id(id)?)?
                    .ok_or(DbError::NotFound("board task"))
            })
            .collect()
    }
}

pub(super) fn contains(root: &VaultPath, path: &VaultPath) -> bool {
    let root = root.collision_key();
    let path = path.collision_key();
    root == path
        || path
            .strip_prefix(&root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn overlaps(left: &[VaultPath], right: &[VaultPath]) -> Option<Vec<String>> {
    let paths: Vec<_> = left
        .iter()
        .filter(|a| right.iter().any(|b| contains(a, b) || contains(b, a)))
        .map(ToString::to_string)
        .collect();
    (!paths.is_empty()).then_some(paths)
}

pub(super) fn require_paths_free(tx: &Transaction<'_>, task_id: TaskId) -> Result<(), DbError> {
    let raw: Option<String> = tx
        .query_row(
            "SELECT definition_json FROM board_tasks WHERE task_id=?1",
            [task_id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(());
    };
    let requested: BoardDefinition = serde_json::from_str(&raw)?;
    require_children_terminal(tx, task_id)?;
    let mut stmt = tx.prepare("SELECT b.definition_json,a.worker_id FROM board_tasks b JOIN attempts a USING(task_id) WHERE a.phase IN ('claimed','running','cancel_requested','committing') AND b.task_id!=?1")?;
    for row in stmt.query_map([task_id.to_string()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (raw, worker) = row?;
        let held: BoardDefinition = serde_json::from_str(&raw)?;
        if let Some(paths) = overlaps(&requested.files_owned, &held.files_owned) {
            return Err(DbError::PathsHeld {
                by_instance: worker,
                paths,
            });
        }
    }
    Ok(())
}

pub(super) fn require_children_terminal(
    tx: &Transaction<'_>,
    task_id: TaskId,
) -> Result<(), DbError> {
    let pending: i64 = tx.query_row("SELECT COUNT(*) FROM board_tasks b JOIN tasks t USING(task_id) WHERE b.parent=?1 AND t.status NOT IN ('succeeded','failed','cancelled','abandoned','approved')",[task_id.to_string()],|r|r.get(0))?;
    if pending != 0 {
        return Err(DbError::BoardRejected("children are not terminal"));
    }
    Ok(())
}
