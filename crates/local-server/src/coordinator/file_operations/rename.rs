//! 人侧只接受明确批准的完整 UTF-8 文本；分类结果绑定同一 Broker 的实际源版本。
use super::*;
use studiolo_core::Revision;
use studiolo_host_config::ApprovedRenamePair;
use studiolo_vault_broker::{
    BrokerError, FileEffect, FileOperationKind, FileOperationOutcome, FileOperationPreview,
    FilePresence,
};

struct Content {
    source: FilePresence,
    target: FilePresence,
}
pub(super) struct RenameAdmission {
    pair: ApprovedRenamePair,
    expected: Revision,
    // 源缺失或不匹配时，只能接受 Broker 固定的 Rejected；不能执行后来出现的未经分类字节。
    content: Option<Content>,
}

impl CanonicalWorkbench {
    pub(super) fn rename_admission(
        &self,
        broker: &FsVaultBroker,
        principal: &ContentPrincipal,
        input: &Input,
    ) -> Result<Option<RenameAdmission>, Problem> {
        let Kind::Rename { expected, .. } = &input.kind else {
            return Ok(None);
        };
        let (_, pair) = self.file_input_access(broker, principal, input)?;
        let pair = pair.ok_or_else(problem::rename_disabled)?;
        if pair.class() != "markdown_text_v1"
            || pair.mime() != "text/markdown; charset=utf-8"
            || pair.extension() != ".md"
        {
            return Err(problem::rename_content());
        }
        let content = match broker.read_file(&input.path, pair.max_bytes().min(4 * 1024 * 1024)) {
            Ok(document) => {
                if document.vault_uuid != pair.vault_uuid() || &document.path != pair.from_path() {
                    return Err(problem::unavailable());
                }
                if document.revision == *expected {
                    let source_revision =
                        Revision::compute(input.vault, &input.path, &document.content);
                    let target_revision =
                        Revision::compute(input.vault, pair.to_path(), &document.content);
                    if source_revision != *expected || source_revision == target_revision {
                        return Err(problem::unavailable());
                    }
                    text(&document.content)?;
                    let sha256 = crate::editing::digest(&document.content);
                    let bytes = document.content.len() as u64;
                    Some(Content {
                        source: FilePresence::Present {
                            revision: source_revision,
                            sha256: sha256.clone(),
                            bytes,
                        },
                        target: FilePresence::Present {
                            revision: target_revision,
                            sha256,
                            bytes,
                        },
                    })
                } else {
                    None
                }
            }
            Err(BrokerError::NotFound(path)) if path == input.path => None,
            Err(error) => return Err(problem::before_issue(error)),
        };
        pair.revalidate().map_err(|_| problem::unavailable())?;
        // 完整 Vec 已释放，只带小证明进入 prepare，随后由 Broker 再核 L0/stage 与双路径 CAS。
        Ok(Some(RenameAdmission {
            pair,
            expected: *expected,
            content,
        }))
    }
}

impl RenameAdmission {
    pub(super) fn ticket(&self, preview: &FileOperationPreview) -> Result<(), Problem> {
        self.pair.revalidate().map_err(|_| problem::unavailable())?;
        if &preview.path != self.pair.from_path()
            || preview.kind
                != (FileOperationKind::Rename {
                    expected: self.expected,
                    to: self.pair.to_path().clone(),
                })
        {
            return Err(problem::unavailable());
        }
        if preview.fixed_outcome == Some(FileOperationOutcome::Rejected) {
            return Ok(());
        }
        let content = self.content.as_ref().ok_or_else(Problem::stale)?;
        if preview.fixed_outcome.is_some() || preview.observed_before != content.source {
            return Err(Problem::stale());
        }
        Ok(())
    }

    pub(super) fn receipt(&self, receipt: &FileOperationReceipt, operation_id: &str) -> bool {
        if receipt.operation_id != operation_id {
            return false;
        }
        if receipt.outcome != FileOperationOutcome::Committed {
            return true;
        }
        let Some(content) = &self.content else {
            return false;
        };
        receipt.tombstone_id.as_deref() == Some(operation_id)
            && receipt.evidence_id.is_none()
            && receipt.effects
                == vec![
                    FileEffect::Confirmed {
                        path: self.pair.to_path().clone(),
                        before: FilePresence::Missing,
                        after: content.target.clone(),
                    },
                    FileEffect::Confirmed {
                        path: self.pair.from_path().clone(),
                        before: content.source.clone(),
                        after: FilePresence::Missing,
                    },
                ]
    }
}

fn text(bytes: &[u8]) -> Result<(), Problem> {
    let value = std::str::from_utf8(bytes).map_err(|_| problem::rename_content())?;
    if value
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n'))
    {
        return Err(problem::rename_content());
    }
    Ok(())
}
