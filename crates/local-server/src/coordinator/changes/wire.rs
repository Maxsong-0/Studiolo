use crate::coordinator::changes::{ObservedChange, ObservedKind};
use serde_json::{Value, json};
use studiolo_vault_broker::ChangeEntry;
pub(super) fn broker(row: &ChangeEntry, unread: Option<bool>) -> Value {
    json!({"id":format!("b:{}:{}",row.sequence,row.id),"source":"broker","kind":row.kind,"path":row.path,"sequence":row.sequence.to_string(),"generation":null,"observation":null,"at_ms":row.at_ms.map(|v|v.to_string()),"actor":row.actor,"previous_revision":row.previous_revision.map(|v|v.to_hex()),"revision":row.revision.map(|v|v.to_hex()),"bytes":row.bytes.to_string(),"counterpart":row.counterpart,"gap":null,"unread":unread})
}
pub(super) fn observation(row: &ObservedChange, epoch: &str, unread: Option<bool>) -> Value {
    json!({"id":format!("o:{epoch}:{}",row.index),"source":"observation","kind":match row.kind {ObservedKind::Created=>"created",ObservedKind::Replaced=>"replaced",ObservedKind::Deleted=>"deleted",ObservedKind::Gap=>"gap"},"path":row.path,"sequence":null,"generation":row.generation.to_string(),"observation":row.index.to_string(),"at_ms":row.at_ms.map(|v|v.to_string()),"actor":null,"previous_revision":row.previous.map(|v|v.to_hex()),"revision":row.revision.map(|v|v.to_hex()),"bytes":row.bytes.map(|v|v.to_string()),"counterpart":null,"gap":row.gap,"unread":unread})
}
