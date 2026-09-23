//! 生成块会保留人工修改，无论改动位于旧系统区之外还是之内。
use crate::board_types::sha256_hex;
use crate::{BoardView, TaskEngineError};
const START: &str = "<!-- studiolo:board:start -->";
const END: &str = "<!-- studiolo:board:end -->";
struct Rendered {
    content: Vec<u8>,
    block_hash: String,
    preserved: Option<String>,
}
fn render(
    prior: &[u8],
    views: &[BoardView],
    known: Option<&str>,
) -> Result<Rendered, TaskEngineError> {
    let text = std::str::from_utf8(prior)
        .map_err(|_| TaskEngineError::Projection("board page is not UTF-8".into()))?;
    let starts: Vec<_> = text.match_indices(START).map(|(i, _)| i).collect();
    let ends: Vec<_> = text.match_indices(END).map(|(i, _)| i).collect();
    let mut block =
        format!("{START}\n\n| Task | State | Holder | Policy | Paths |\n|---|---|---|---|---|\n");
    for view in views {
        let state = serde_json::to_value(view.task.status)?
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
        let holder = view
            .holder
            .as_deref()
            .unwrap_or("—")
            .replace(['|', '\n', '\r'], " ");
        let paths = view
            .board
            .definition
            .files_owned
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
            .replace('|', "\\|");
        block.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            view.task.task_id,
            state,
            holder,
            if view.uses_old_policy {
                "old"
            } else {
                "current"
            },
            paths
        ));
    }
    block.push_str(&format!("\n{END}"));
    let block_hash = sha256_hex(block.as_bytes());
    let mut preserved = None;
    let result = match (starts.as_slice(), ends.as_slice()) {
        ([], []) => format!(
            "{text}{}{block}\n",
            if text.is_empty() || text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            }
        ),
        ([start], [end]) if start < end => {
            let old = &text[*start..*end + END.len()];
            let migration = if known != Some(sha256_hex(old.as_bytes()).as_str()) {
                preserved = Some(sha256_hex(old.as_bytes()));
                format!(
                    "<!-- studiolo:preserved-board-edit:{} -->\n\n## 保留的带外黑板修改\n{}\n\n",
                    sha256_hex(old.as_bytes()),
                    &text[*start + START.len()..*end]
                )
            } else {
                String::new()
            };
            format!(
                "{}{}{}{}",
                &text[..*start],
                migration,
                block,
                &text[*end + END.len()..]
            )
        }
        _ => {
            return Err(TaskEngineError::Projection(
                "ambiguous generated board markers".into(),
            ));
        }
    };
    Ok(Rendered {
        content: result.into_bytes(),
        block_hash,
        preserved,
    })
}

impl<
    B: studiolo_vault_broker::VaultBroker
        + studiolo_vault_broker::ArtifactStager
        + studiolo_vault_broker::BatchWriter,
    L: studiolo_audit_ledger::AuditLedger,
> crate::PersistentTaskEngine<B, L>
{
    /// 经 Broker 批量写入投影一个生成块；被改动过的旧块移入人工小节。
    pub fn project_board(
        &mut self,
        path: &studiolo_core::VaultPath,
        project: &str,
        current_policy_hash: &str,
    ) -> Result<(), TaskEngineError> {
        use studiolo_audit_ledger::AuditPhase;
        use studiolo_vault_broker::{BatchChange, BatchEdit, BatchRequest, BrokerError, TextPatch};
        let through = self
            .control_db
            .board_projection_sequence()
            .map_err(crate::map_db)?;
        let views = self.board_list(project, current_policy_hash)?;
        let key = path.collision_key();
        let known = self
            .control_db
            .board_projection_hash(&key)
            .map_err(crate::map_db)?;
        let old = match self.broker.read(path) {
            Ok(object) => Some(object),
            Err(BrokerError::NotFound(_)) => None,
            Err(e) => return Err(TaskEngineError::Broker(e)),
        };
        let prior = old
            .as_ref()
            .map(|o| o.content.as_slice())
            .unwrap_or_default();
        let rendered = render(prior, &views, known.as_deref())?;
        if rendered.content != prior {
            let after = String::from_utf8(rendered.content)
                .map_err(|_| TaskEngineError::Projection("generated board is not UTF-8".into()))?;
            let edit = if let Some(old) = old {
                let before = String::from_utf8(old.content)
                    .map_err(|_| TaskEngineError::Projection("board page is not UTF-8".into()))?;
                if before.is_empty() {
                    return Err(TaskEngineError::Projection(
                        "existing empty board page has no unique patch anchor".into(),
                    ));
                }
                BatchEdit::Patch {
                    expected: old.revision,
                    patches: vec![TextPatch { before, after }],
                }
            } else {
                BatchEdit::Create { content: after }
            };
            let request = BatchRequest {
                batch_id: format!("board-projection:{}", ulid::Ulid::generate()),
                changes: vec![BatchChange {
                    path: path.clone(),
                    edit,
                }],
            };
            let preview = self
                .broker
                .preview_batch(&request)
                .map_err(TaskEngineError::Broker)?;
            self.audit_board(
                "task.board_projected",
                &(request.batch_id.as_str(), preview.fingerprint.as_str()),
                AuditPhase::Intent,
                None,
            )?;
            let result = self
                .broker
                .commit_batch(request.clone(), &preview)
                .map_err(TaskEngineError::Broker);
            self.audit_board(
                "task.board_projected",
                &(request.batch_id.as_str(), preview.fingerprint.as_str()),
                AuditPhase::Outcome,
                Some(result.is_ok()),
            )?;
            result?;
        }
        self.control_db
            .record_board_projection(
                &key,
                &rendered.block_hash,
                project,
                through,
                rendered.preserved.as_deref(),
            )
            .map_err(crate::map_db)?;
        self.deliver_runtime_audits()?;
        Ok(())
    }
}
