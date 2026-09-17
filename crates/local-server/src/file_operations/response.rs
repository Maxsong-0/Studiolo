//! 公开响应只含逻辑事实；revision 为 hex、sequence 为字符串，不泄露私有存储位置。
use serde_json::{Value, json};
use studiolo_vault_broker::{FileEffect, FileOperationReceipt, FilePresence, TombstonePage};

pub(super) fn saved(value: FileOperationReceipt) -> Value {
    json!({"status":"committed","receipt":{
        "operation_id":value.operation_id,"sequence":value.sequence.to_string(),
        "outcome":value.outcome,"effects":value.effects.iter().map(effect).collect::<Vec<_>>(),
        "tombstone_id":value.tombstone_id,"evidence_id":value.evidence_id
    }})
}

fn effect(value: &FileEffect) -> Value {
    match value {
        FileEffect::Confirmed {
            path,
            before,
            after,
        } => json!({"kind":"confirmed", "path":path,
            "before":presence(before),"after":presence(after)}),
        FileEffect::NotApplied { path } => json!({"kind":"not_applied","path":path}),
        FileEffect::Unknown { path } => json!({"kind":"unknown","path":path}),
    }
}
fn presence(value: &FilePresence) -> Value {
    match value {
        FilePresence::Missing => json!({"kind":"missing"}),
        FilePresence::Present {
            revision,
            sha256,
            bytes,
        } => json!({"kind":"present","revision":revision.to_hex(),"sha256":sha256,"bytes":bytes}),
    }
}
pub(super) fn page(value: TombstonePage) -> Value {
    let entries: Vec<_> = value
        .entries
        .into_iter()
        .map(|entry| {
            json!({
                "id":entry.id,"path":entry.path,"revision":entry.revision.to_hex(),
                "sha256":entry.sha256,"bytes":entry.bytes,"deleted_at_ms":entry.deleted_at_ms,
                "deleted_by":entry.deleted_by,"retain_until_ms":entry.retain_until_ms
            })
        })
        .collect();
    json!({"entries":entries,"next_cursor":value.next_cursor})
}
