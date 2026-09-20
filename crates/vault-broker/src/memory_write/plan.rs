//! 路径边界、日期、单条预算和 YAML 规范编码。
use super::{MemoryEntryRequest, MemoryPreview, fold};
use crate::{
    BatchChange, BatchEdit, BatchRequest, BatchWriter, BrokerError, VaultObject, VaultPath,
};
use serde_json::Value;
pub(super) fn invalid(message: &str) -> BrokerError {
    BrokerError::InvalidArtifact(message.into())
}
pub(super) fn key(kind: &str, summary: &str) -> Result<String, BrokerError> {
    if kind.len() > 512 || summary.len() > 4096 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let kind = fold::normalize(kind);
    let summary = fold::normalize(summary);
    if kind.is_empty()
        || summary.is_empty()
        || kind.len() > 128
        || summary.len() > 1024
        || kind.contains(['\n', '\r', '\0'])
        || summary.contains(['\n', '\r', '\0'])
    {
        return Err(invalid("记忆类型或摘要无效"));
    }
    crate::FsVaultBroker::fingerprint(&("studiolo-memory-v1", kind, summary))
}
pub(super) fn scope(scope: &VaultPath) -> Result<(), BrokerError> {
    if scope.is_protected() || !scope.as_str().split('/').any(|part| part == "03-记忆") {
        return Err(invalid("记忆 scope 必须位于 03-记忆"));
    }
    if scope.as_str().len() > 512 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok(())
}
pub(super) fn date(value: &str) -> Result<(), BrokerError> {
    let date = value.as_bytes();
    if date.len() != 10
        || date[4] != b'-'
        || date[7] != b'-'
        || date
            .iter()
            .enumerate()
            .any(|(i, c)| i != 4 && i != 7 && !c.is_ascii_digit())
        || value.parse::<jiff::civil::Date>().is_err()
    {
        return Err(invalid("记忆日期必须是有效 YYYY-MM-DD 绝对日期"));
    }
    Ok(())
}
pub(super) fn content(request: &MemoryEntryRequest) -> Result<(String, String), BrokerError> {
    scope(&request.scope)?;
    if request.batch_id.is_empty()
        || request.batch_id.len() > 256
        || request.body.len() > 256 * 1024
        || request.body.contains('\0')
    {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let key = key(&request.kind, &request.summary)?;
    date(&request.date)?;
    let quote =
        |value: &str| serde_json::to_string(value).map_err(|_| invalid("记忆 YAML 编码失败"));
    let body = request.body.trim_end_matches(['\n', '\r']);
    if body.trim().is_empty() {
        return Err(invalid("记忆正文不能为空"));
    }
    let text = format!(
        "---\ntype: {}\nsummary: {}\ndate: {}\ndedupe_key: {}\n---\n{}\n",
        quote(request.kind.trim())?,
        quote(request.summary.trim())?,
        quote(&request.date)?,
        quote(&key)?,
        body
    );
    if text.lines().count() > 300 {
        return Err(BrokerError::WriterLimitExceeded);
    }
    Ok((key, text))
}
pub(super) fn frontmatter(bytes: &[u8]) -> Result<Option<Value>, BrokerError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("既有记忆 Markdown 非 UTF-8"))?;
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return Ok(None);
    };
    let mut yaml = String::new();
    let mut closed = false;
    for line in rest.lines() {
        if line == "---" || line == "..." {
            closed = true;
            break;
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    if !closed {
        return Err(invalid("既有记忆 frontmatter 未闭合"));
    }
    let value: Value =
        serde_saphyr::from_str(&yaml).map_err(|_| invalid("既有记忆 frontmatter 无效"))?;
    Ok(Some(value))
}
pub(super) fn existing_key(bytes: &[u8]) -> Result<Option<String>, BrokerError> {
    let Some(value) = frontmatter(bytes)? else {
        return Ok(None);
    };
    let (Some(kind), Some(summary)) = (value.get("type"), value.get("summary")) else {
        return Ok(None);
    };
    let (Some(kind), Some(summary)) = (kind.as_str(), summary.as_str()) else {
        return Err(invalid("既有记忆类型和摘要必须为文本"));
    };
    key(kind, summary).map(Some)
}
pub(super) fn preview(
    broker: &impl BatchWriter,
    request: &MemoryEntryRequest,
    records: Vec<VaultObject>,
) -> Result<MemoryPreview, BrokerError> {
    let (key, text) = content(request)?;
    let mut found = None;
    for object in records {
        if existing_key(&object.content)?.as_deref() == Some(key.as_str()) {
            if found.is_some() {
                return Err(invalid("范围内已有多条重复记忆，须先人工归并"));
            }
            found = Some(MemoryPreview::Existing {
                sha256: crate::sha256(&object.content),
                path: object.path,
                revision: object.revision,
                requires_patch: true,
            });
        }
    }
    if let Some(existing) = found {
        return Ok(existing);
    }
    let path = VaultPath::parse(format!("{}/memory-{key}.md", request.scope))
        .map_err(|_| invalid("派生记忆路径无效"))?;
    let batch = BatchRequest {
        batch_id: request.batch_id.clone(),
        changes: vec![BatchChange {
            path,
            edit: BatchEdit::Create { content: text },
        }],
    };
    let preview = broker.preview_batch(&batch)?;
    Ok(MemoryPreview::Create {
        request: batch,
        preview,
    })
}
