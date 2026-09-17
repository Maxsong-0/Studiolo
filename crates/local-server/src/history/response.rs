//! HTTP显式投影历史元数据；首次未知作者不由归档主体或磁盘mtime代填。
use serde_json::{Value, json};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_vault_broker::{HistoryPage, MutationActor};

fn actor(value: Option<MutationActor>) -> Value {
    match value {
        Some(MutationActor::Human { issuer, subject }) => {
            json!({"kind":"human", "issuer":issuer, "subject":subject})
        }
        Some(MutationActor::Obsidian) => json!({"kind":"obsidian"}),
        None => Value::Null,
    }
}
pub(super) fn page(vault: VaultUuid, path: VaultPath, page: HistoryPage) -> Value {
    let entries: Vec<_> = page
        .entries
        .into_iter()
        .map(|entry| {
            json!({
                "id":entry.id,"revision":entry.revision.to_hex(),"bytes":entry.bytes,
                "authored_at_ms":entry.authored_at_ms,"author":actor(entry.author),
                "archived_at_ms":entry.archived_at_ms,"archived_by":actor(entry.archived_by),
            })
        })
        .collect();
    json!({"vault_uuid":vault.to_string(), "path":path.to_string(),
        "entries":entries,"next_cursor":page.next_cursor})
}
