//! 所有内容写回需要同一有效 claim；只能触达该项目与声明的独占路径交集。
use super::{
    Engine,
    auth::{Grant, within},
    requests::{Identity, Request},
};
use serde::Deserialize;
use serde_json::Value;
use studiolo_vault_broker::{
    BatchPreview, BatchRequest, BatchWriter, MemoryEntryRequest, MemoryPreview, MemoryWriter,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Batch {
    identity: Identity,
    batch: BatchRequest,
    preview: Option<BatchPreview>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rollback {
    identity: Identity,
    batch_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Memory {
    identity: Identity,
    entry: MemoryEntryRequest,
}

fn key(r: &Request, i: &Identity, id: &str) -> Result<String, String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("批次ID必须为1至64位字母数字或连字符".into());
    }
    Ok(format!("{}-{}-{id}", r.instance_id, i.task_id))
}
fn allowed(
    engine: &Engine,
    r: &Request,
    g: &Grant,
    i: &Identity,
    paths: impl IntoIterator<Item = studiolo_core::VaultPath>,
    now: i64,
) -> Result<(), String> {
    super::execute::owned(engine, r, i, g, now, false)?;
    if !g.set.write.content_patch_only || g.set.write.content_direct {
        return Err("内容写回需要patch-only能力".into());
    }
    let view = engine
        .board_detail(i.task_id, &g.hash)
        .map_err(|e| e.to_string())?
        .ok_or("工作项不存在")?;
    for path in paths {
        if !within(&path, &r.project)
            || !view
                .board
                .definition
                .files_owned
                .iter()
                .any(|p| within(&path, p))
        {
            return Err("内容路径不在项目与本项独占路径交集内".into());
        }
    }
    Ok(())
}
pub(super) fn run(engine: &mut Engine, r: &Request, g: &Grant, now: i64) -> Result<Value, String> {
    match r.operation.as_str() {
        "batch.preview" | "batch.commit" => {
            let mut a: Batch = super::execute::parse(&r.request)?;
            allowed(
                engine,
                r,
                g,
                &a.identity,
                a.batch.changes.iter().map(|c| c.path.clone()),
                now,
            )?;
            a.batch.batch_id = key(r, &a.identity, &a.batch.batch_id)?;
            engine
                .broker()
                .access(|broker| {
                    if r.operation == "batch.preview" {
                        let preview = broker.preview_batch(&a.batch)?;
                        Ok(serde_json::json!({"preview":preview}))
                    } else {
                        let preview = a
                            .preview
                            .as_ref()
                            .ok_or(studiolo_vault_broker::BrokerError::CorruptWriterState)?;
                        let receipt = broker.commit_batch(a.batch, preview)?;
                        Ok(serde_json::json!({"receipt":receipt}))
                    }
                })
                .map_err(|e| e.to_string())
        }
        "batch.rollback" => {
            let a: Rollback = super::execute::parse(&r.request)?;
            allowed(engine, r, g, &a.identity, std::iter::empty(), now)?;
            let id = key(r, &a.identity, &a.batch_id)?;
            engine
                .broker()
                .access(|broker| broker.rollback_batch(&id))
                .map_err(|e| e.to_string())
                .and_then(super::execute::value)
        }
        "memory.preview" => {
            let mut a: Memory = super::execute::parse(&r.request)?;
            allowed(engine, r, g, &a.identity, [a.entry.scope.clone()], now)?;
            let memory = studiolo_core::VaultPath::parse(format!("{}/03-记忆", r.project))
                .map_err(|_| "记忆路径无效")?;
            if !within(&a.entry.scope, &memory) {
                return Err("记忆只能进入本项目03-记忆".into());
            }
            let raw_id = a.entry.batch_id.clone();
            a.entry.batch_id = key(r, &a.identity, &a.entry.batch_id)?;
            let mut result = engine
                .broker()
                .access(|broker| broker.preview_memory(&a.entry))
                .map_err(|e| e.to_string())?;
            if let MemoryPreview::Create { request, .. } = &mut result {
                request.batch_id = raw_id;
            }
            super::execute::value(result)
        }
        _ => Err("未知内容操作".into()),
    }
}
